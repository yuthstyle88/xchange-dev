//! LG-6.r6 (#1046; ledger design #553 `docs/design/blueprint/ledger-inventory.md` §4.2.1, ruling A): after
//! CA-10 nothing writes `stock_movements`, so its readers list `cash_transfers`. Written before the
//! implementation by the ticket author (claude/d7aa64). Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! What is locked, row by row (ticket §3.1, §3.2, §4, §9):
//! - `GET /api/v3/views/admin_cash_movements` lists the branch's `cash_transfers`, one row per
//!   (transfer, currency), in the same row shape `{id, created, shiftId, direction, currency, amount, reason,
//!   staff}`:
//!   - `id` = `<transfer id>:<currency>`; `created` = `dispatched_at`; `staff` = `dispatched_by`;
//!   - `direction`: the drawer is the reference. `OUT` for `DRAWER_TO_SAFE`, `EXTERNAL_OUT` and a `BRANCH`
//!     transfer on the sender; `IN` for `SAFE_TO_DRAWER`, `EXTERNAL_IN` and a `BRANCH` transfer on the receiver;
//!   - `shiftId` = `from_shift_id` for OUT, `to_shift_id` for IN, `""` when none;
//!   - `amount` = the transfer's amount in that currency (face value × quantity sent), two places, a string;
//!   - `reason` = `note`, `""` when NULL;
//!   - only `DISPATCHED`, `RECEIVED` and `RECONCILED` transfers are listed;
//!   - keyset paging with `before`: no row repeated or skipped, newest dispatched first.
//! - The v2 snapshot's `stock.movements` follows the same mapping (§3.2).
//! - `shift_valuation` takes the shift's transfers (§3.2): a currency the shift moved is listed even when its
//!   opening and expected are zero; a transfer that is not the shift's, or not moved yet, adds nothing.
//! - The source scan of the old body's done criterion 2 now lives in `lg6_r6b_no_stock_movements_test.rs` (#1046 §12 Q4).
//! - A `BRANCH` transfer that is only `DISPATCHED` is OUT on the sender and absent on the receiver; it is IN on the
//!   receiver only once `RECEIVED` or later (coordinator's ruling for #1046, 2026-10-15, from
//!   `ledger-inventory.md:311-312`: at dispatch the money sits in the sender's IN_TRANSIT; the receiver gets it
//!   only at receive).
//!
//! Not locked here, on purpose: the order of the currency
//! rows inside one transfer; the snapshot's `requestId` for a transfer row (no ruling); the abandon path
//! (`compat/abandon_request.rs`, ticket §3.3: CA-10's).
//!
//! Transfers are seeded straight into `cash_transfers` / `cash_transfer_lines` (the writers are CA-5/6's and
//! are not what this ticket changes), each with its own `dispatched_at`, so nothing relies on two writes having
//! different clock readings. Shifts are opened by count (`shift_setup_common`). Needs `TEST_DATABASE_URL`
//! (see `snapshot_common/mod.rs`); nothing skips.

mod common;
mod compat_common;
mod rpc_common;
mod shift_setup_common;
mod snapshot_common;

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";
const RPC: &str = "/api/rpc";
const MOVEMENTS: &str = "admin_cash_movements";
const VALUATION: &str = "/api/v3/compute/shift_valuation";
const MAIN: &str = "MAIN";
const B9: &str = "B9";

const ROW_FIELDS: [&str; 8] = [
    "id",
    "created",
    "shiftId",
    "direction",
    "currency",
    "amount",
    "reason",
    "staff",
];

fn iso(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn list(value: &Value) -> &Vec<Value> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("a list: {value}"))
}

fn ids(rows: &Value) -> Vec<String> {
    list(rows)
        .iter()
        .map(|r| {
            r["id"]
                .as_str()
                .unwrap_or_else(|| panic!("a row's id is a string: {r}"))
                .to_owned()
        })
        .collect()
}

/// The rows of `rows` that belong to transfer `tid` (`<tid>:<currency>`).
fn of_transfer(rows: &Value, tid: &str) -> Vec<Value> {
    let prefix = format!("{tid}:");
    list(rows)
        .iter()
        .filter(|r| r["id"].as_str().is_some_and(|id| id.starts_with(&prefix)))
        .cloned()
        .collect()
}

fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("an object: {value}"))
        .keys()
        .cloned()
        .collect()
}

/// The row §3.1 maps a transfer's currency to.
#[allow(clippy::too_many_arguments)]
fn row(
    tid: &str,
    currency: &str,
    created: DateTime<Utc>,
    shift: &str,
    direction: &str,
    amount: &str,
    reason: &str,
    staff: &str,
) -> Value {
    json!({
        "id": format!("{tid}:{currency}"),
        "created": iso(created),
        "shiftId": shift,
        "direction": direction,
        "currency": currency,
        "amount": amount,
        "reason": reason,
        "staff": staff,
    })
}

// ── seeding ──────────────────────────────────────────────────────────────────

/// One transfer as the CA-5/6 flow leaves it. `lines` are `(currency, face value, quantity)`.
struct Seed<'a> {
    kind: &'a str,
    from: &'a str,
    to: Option<&'a str>,
    from_shift: Option<&'a str>,
    to_shift: Option<&'a str>,
    status: &'a str,
    lines: &'a [(&'a str, &'a str, i32)],
    note: Option<&'a str>,
    /// `dispatched_at` for DISPATCHED and later; ignored before.
    at: DateTime<Utc>,
}

/// A user of the organisation; answers its id.
async fn user(pool: &sqlx::PgPool, name: &str) -> String {
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         SELECT id, $1, 'PENDING', 'test' FROM organizations WHERE code = 'XC' RETURNING id::text",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("seed user {name}: {e}"))
}

/// Inserts `seed`, created by `creator` an hour before `seed.at`, dispatched by `sender` at `seed.at`, received
/// (in full) by `receiver` a minute later. Answers the transfer's id.
async fn seed_transfer(
    pool: &sqlx::PgPool,
    seed: &Seed<'_>,
    creator: &str,
    sender: &str,
    receiver: &str,
) -> String {
    let dispatched = matches!(seed.status, "DISPATCHED" | "RECEIVED" | "RECONCILED");
    let received = matches!(seed.status, "RECEIVED" | "RECONCILED");
    let mut tx = pool.begin().await.expect("begin");
    let id: String = sqlx::query_scalar(
        "INSERT INTO cash_transfers (organization_id, kind, from_branch_id, to_branch_id, from_shift_id, \
             to_shift_id, status, created_by, dispatched_at, dispatched_by, received_at, received_by, \
             reconciled_at, reconciled_by, note, idempotency_key, created_at, updated_at) \
         SELECT o.id, $1, $2, $3, $4, $5, $6, $7::uuid, $8, $9::uuid, $10, $11::uuid, $12, $13::uuid, $14, \
             gen_random_uuid()::text, $15, $15 \
         FROM organizations o WHERE o.code = 'XC' RETURNING id::text",
    )
    .bind(seed.kind)
    .bind(seed.from)
    .bind(seed.to)
    .bind(seed.from_shift)
    .bind(seed.to_shift)
    .bind(seed.status)
    .bind(creator)
    .bind(dispatched.then_some(seed.at))
    .bind(dispatched.then_some(sender))
    .bind(received.then_some(seed.at + Duration::minutes(1)))
    .bind(received.then_some(receiver))
    .bind((seed.status == "RECONCILED").then_some(seed.at + Duration::minutes(2)))
    .bind((seed.status == "RECONCILED").then_some(receiver))
    .bind(seed.note)
    .bind(seed.at - Duration::hours(1))
    .fetch_one(&mut *tx)
    .await
    .unwrap_or_else(|e| panic!("seed a {} transfer: {e}", seed.kind));
    for (currency, face, qty) in seed.lines {
        sqlx::query(
            "INSERT INTO cash_transfer_lines (transfer_id, organization_id, currency, denomination_id, qty_sent, \
                 qty_received) \
             SELECT $1::uuid, t.organization_id, $2, d.id, $4, $5 \
             FROM cash_transfers t, denominations d \
             WHERE t.id = $1::uuid AND d.currency_code = $2 AND d.face_value = $3::numeric",
        )
        .bind(&id)
        .bind(*currency)
        .bind(*face)
        .bind(*qty)
        .bind(received.then_some(*qty))
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|e| panic!("seed a line {currency} {face} × {qty}: {e}"));
    }
    for branch in [Some(seed.from), seed.to].into_iter().flatten() {
        sqlx::query(
            "INSERT INTO branch_versions (branch_id, version, changed_at) VALUES ($1, 1, $2) \
             ON CONFLICT (branch_id) DO UPDATE SET version = branch_versions.version + 1, changed_at = $2",
        )
        .bind(branch)
        .bind(seed.at)
        .execute(&mut *tx)
        .await
        .expect("bump branch_versions");
    }
    tx.commit().await.expect("commit the transfer");
    id
}

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
    creator: String,
    sender: String,
    receiver: String,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        desk.db.seed_branch(B9, "Nine").await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let t0 = snapshot_common::time("2026-10-06T05:00:00.000Z");
        desk.clock.set(t0);
        let pool = desk.db.pool.clone();
        Room {
            creator: user(&pool, "r6-creator").await,
            sender: user(&pool, "r6-sender").await,
            receiver: user(&pool, "r6-receiver").await,
            desk,
            app,
            cookies,
            t0,
        }
    }

    fn pool(&self) -> &sqlx::PgPool {
        &self.desk.db.pool
    }

    async fn open_shift(&self, branch: &str) -> String {
        shift_setup_common::open_shift_by_count(
            self.pool(),
            branch,
            json!({ "THB": 100000, "USD": 1000 }),
            self.t0,
        )
        .await
    }

    async fn transfer(&self, seed: Seed<'_>) -> String {
        seed_transfer(
            self.pool(),
            &seed,
            &self.creator,
            &self.sender,
            &self.receiver,
        )
        .await
    }

    async fn send(&self, call: Call) -> Answer {
        self.app.send(call.cookie(&self.cookies["OWNER"])).await
    }

    /// The owner's `admin_cash_movements` rows for `query`.
    async fn movements(&self, query: &str) -> Value {
        let mut call = Call::post(
            ORIGIN,
            &format!("/api/v3/views/{MOVEMENTS}?{query}"),
            Vec::<u8>::new(),
        )
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
        call.method = actix_web::http::Method::GET;
        let answer = self.send(call).await;
        assert_eq!(
            answer.status, 200,
            "GET {MOVEMENTS}?{query}: {:.400}",
            answer.text
        );
        let body = answer.json();
        assert_eq!(body["changed"], json!(true), "{query}: {body}");
        body["data"].clone()
    }

    async fn snapshot(&self, branch: &str) -> Value {
        let answer = self
            .send(Call::action(
                ORIGIN,
                RPC,
                "adminSnapshot",
                json!({ "branchId": branch }),
            ))
            .await;
        assert_eq!(answer.status, 200, "adminSnapshot: {:.400}", answer.text);
        answer.json()
    }

    async fn valuation(&self, branch: &str, shift: &str) -> Value {
        let body = json!({ "branch": branch, "shiftId": shift }).to_string();
        let answer = self.send(Call::post(ORIGIN, VALUATION, body)).await;
        assert_eq!(answer.status, 200, "shift_valuation: {:.400}", answer.text);
        answer.json()
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

// ── §9 row 1: a DRAWER_TO_SAFE of 500 THB ────────────────────────────────────

#[actix_web::test]
async fn a_dispatched_drawer_to_safe_is_one_out_row_of_its_shift_with_the_note() {
    let room = Room::open().await;
    let shift = room.open_shift(MAIN).await;
    let at = room.t0 + Duration::minutes(10);
    let tid = room
        .transfer(Seed {
            kind: "DRAWER_TO_SAFE",
            from: MAIN,
            to: None,
            from_shift: Some(&shift),
            to_shift: None,
            status: "DISPATCHED",
            lines: &[("THB", "100", 5)],
            note: Some("ส่งเงินเข้าเซฟ"),
            at,
        })
        .await;

    let rows = room.movements("branch=MAIN").await;
    let mine = of_transfer(&rows, &tid);
    assert_eq!(
        mine,
        vec![row(
            &tid,
            "THB",
            at,
            &shift,
            "OUT",
            "500.00",
            "ส่งเงินเข้าเซฟ",
            &room.sender
        )],
        "one row per (transfer, currency), in §3.1's mapping: {rows}"
    );
    assert_eq!(
        keys(&mine[0]),
        ROW_FIELDS.iter().map(|s| s.to_string()).collect(),
        "the row shape is unchanged"
    );
    room.finish().await;
}

// ── §3.1 direction / shiftId for the single-branch kinds ─────────────────────

#[actix_web::test]
async fn each_single_branch_kind_takes_the_drawer_as_its_reference() {
    let room = Room::open().await;
    let shift = room.open_shift(MAIN).await;
    let t = |m: i64| room.t0 + Duration::minutes(m);
    let to_drawer = room
        .transfer(Seed {
            kind: "SAFE_TO_DRAWER",
            from: MAIN,
            to: None,
            from_shift: None,
            to_shift: Some(&shift),
            status: "RECEIVED",
            lines: &[("USD", "100", 3)],
            note: Some("เติมลิ้นชัก"),
            at: t(1),
        })
        .await;
    let ext_in = room
        .transfer(Seed {
            kind: "EXTERNAL_IN",
            from: MAIN,
            to: None,
            from_shift: None,
            to_shift: None,
            status: "RECEIVED",
            lines: &[("THB", "1000", 2)],
            note: Some("รับจากธนาคาร"),
            at: t(2),
        })
        .await;
    let ext_out = room
        .transfer(Seed {
            kind: "EXTERNAL_OUT",
            from: MAIN,
            to: None,
            from_shift: None,
            to_shift: None,
            status: "RECONCILED",
            lines: &[("THB", "500", 1)],
            note: Some("ส่งธนาคาร"),
            at: t(3),
        })
        .await;

    let rows = room.movements("branch=MAIN").await;
    assert_eq!(
        of_transfer(&rows, &to_drawer),
        vec![row(
            &to_drawer,
            "USD",
            t(1),
            &shift,
            "IN",
            "300.00",
            "เติมลิ้นชัก",
            &room.sender
        )],
        "SAFE_TO_DRAWER is IN, on to_shift_id: {rows}"
    );
    assert_eq!(
        of_transfer(&rows, &ext_in),
        vec![row(
            &ext_in,
            "THB",
            t(2),
            "",
            "IN",
            "2000.00",
            "รับจากธนาคาร",
            &room.sender
        )],
        "EXTERNAL_IN is IN, no shift: {rows}"
    );
    assert_eq!(
        of_transfer(&rows, &ext_out),
        vec![row(
            &ext_out,
            "THB",
            t(3),
            "",
            "OUT",
            "500.00",
            "ส่งธนาคาร",
            &room.sender
        )],
        "EXTERNAL_OUT is OUT, no shift; RECONCILED is listed: {rows}"
    );
    room.finish().await;
}

// ── §9 row 2: a BRANCH transfer B1 → B2 with USD and EUR ─────────────────────

#[actix_web::test]
async fn a_branch_transfer_is_out_on_the_sender_and_in_on_the_receiver_once_received() {
    let room = Room::open().await;
    let at = room.t0 + Duration::minutes(5);
    let seed = |status| Seed {
        kind: "BRANCH",
        from: MAIN,
        to: Some(B9),
        from_shift: None,
        to_shift: None,
        status,
        lines: &[("USD", "100", 2), ("EUR", "50", 3)],
        note: Some("โอนไปสาขา 9"),
        at,
    };
    let tid = room.transfer(seed("DISPATCHED")).await;
    let sorted = |mut v: Vec<Value>| {
        v.sort_by_key(|r| r["id"].as_str().unwrap_or_default().to_owned());
        v
    };

    let main = room.movements("branch=MAIN").await;
    assert_eq!(
        sorted(of_transfer(&main, &tid)),
        vec![
            row(
                &tid,
                "EUR",
                at,
                "",
                "OUT",
                "150.00",
                "โอนไปสาขา 9",
                &room.sender
            ),
            row(
                &tid,
                "USD",
                at,
                "",
                "OUT",
                "200.00",
                "โอนไปสาขา 9",
                &room.sender
            ),
        ],
        "the sender lists one OUT row per currency: {main}"
    );
    let nine = room.movements("branch=B9").await;
    assert_eq!(
        of_transfer(&nine, &tid),
        Vec::<Value>::new(),
        "in transit (DISPATCHED, not received): nothing on the receiver yet: {nine}"
    );

    sqlx::query(
        "UPDATE cash_transfers SET status = 'RECEIVED', received_at = $2, received_by = $3::uuid WHERE id = $1::uuid",
    )
    .bind(&tid)
    .bind(at + Duration::minutes(30))
    .bind(&room.receiver)
    .execute(room.pool())
    .await
    .expect("receive");
    sqlx::query(
        "UPDATE cash_transfer_lines SET qty_received = qty_sent WHERE transfer_id = $1::uuid",
    )
    .bind(&tid)
    .execute(room.pool())
    .await
    .expect("count in full");

    let nine = room.movements("branch=B9").await;
    assert_eq!(
        sorted(of_transfer(&nine, &tid)),
        vec![
            row(
                &tid,
                "EUR",
                at,
                "",
                "IN",
                "150.00",
                "โอนไปสาขา 9",
                &room.sender
            ),
            row(
                &tid,
                "USD",
                at,
                "",
                "IN",
                "200.00",
                "โอนไปสาขา 9",
                &room.sender
            ),
        ],
        "the receiver lists one IN row per currency once received: {nine}"
    );
    let main = room.movements("branch=MAIN").await;
    assert_eq!(
        of_transfer(&main, &tid).len(),
        2,
        "the sender still lists its two OUT rows after the receipt: {main}"
    );
    assert!(
        of_transfer(&main, &tid)
            .iter()
            .all(|r| r["direction"] == json!("OUT")),
        "{main}"
    );
    room.finish().await;
}

// ── §9 row 3: DRAFT (and APPROVED) are not moves yet ─────────────────────────

#[actix_web::test]
async fn draft_and_approved_transfers_are_not_moves_yet() {
    let room = Room::open().await;
    let shift = room.open_shift(MAIN).await;
    let t = |m: i64| room.t0 + Duration::minutes(m);
    let drawer_out = |status, m| Seed {
        kind: "DRAWER_TO_SAFE",
        from: MAIN,
        to: None,
        from_shift: Some(&shift),
        to_shift: None,
        status,
        lines: &[("THB", "100", 1)],
        note: Some("x"),
        at: t(m),
    };
    let draft = room.transfer(drawer_out("DRAFT", 1)).await;
    let approved = room.transfer(drawer_out("APPROVED", 2)).await;
    let dispatched = room.transfer(drawer_out("DISPATCHED", 3)).await;

    let rows = room.movements("branch=MAIN").await;
    assert_eq!(
        of_transfer(&rows, &draft),
        Vec::<Value>::new(),
        "DRAFT: {rows}"
    );
    assert_eq!(
        of_transfer(&rows, &approved),
        Vec::<Value>::new(),
        "APPROVED: {rows}"
    );
    assert_eq!(
        ids(&Value::Array(of_transfer(&rows, &dispatched))),
        vec![format!("{dispatched}:THB")],
        "DISPATCHED is listed: {rows}"
    );
    room.finish().await;
}

// ── §9 row 4: a NULL note ────────────────────────────────────────────────────

#[actix_web::test]
async fn a_null_note_is_an_empty_reason() {
    let room = Room::open().await;
    let shift = room.open_shift(MAIN).await;
    let tid = room
        .transfer(Seed {
            kind: "DRAWER_TO_SAFE",
            from: MAIN,
            to: None,
            from_shift: Some(&shift),
            to_shift: None,
            status: "DISPATCHED",
            lines: &[("USD", "100", 1)],
            note: None,
            at: room.t0 + Duration::minutes(1),
        })
        .await;

    let rows = room.movements("branch=MAIN").await;
    let mine = of_transfer(&rows, &tid);
    assert_eq!(mine.len(), 1, "{rows}");
    assert_eq!(
        mine[0]["reason"],
        json!(""),
        "NULL note → \"\", never null: {rows}"
    );
    room.finish().await;
}

// ── §9 row 5: paging with `before` ───────────────────────────────────────────

#[actix_web::test]
async fn paging_with_before_neither_repeats_nor_skips_a_row() {
    let room = Room::open().await;
    let shift = room.open_shift(MAIN).await;
    let t = |m: i64| room.t0 + Duration::minutes(m);
    let out = |m, lines| Seed {
        kind: "DRAWER_TO_SAFE",
        from: MAIN,
        to: None,
        from_shift: Some(&shift),
        to_shift: None,
        status: "DISPATCHED",
        lines,
        note: Some("p"),
        at: t(m),
    };
    // dispatched in this order; seeded in another, so insertion order is not the answer
    let newest = room.transfer(out(4, &[("THB", "100", 1)])).await;
    let oldest = room.transfer(out(1, &[("THB", "100", 2)])).await;
    let two_currencies = room
        .transfer(out(2, &[("THB", "100", 3), ("USD", "100", 1)]))
        .await;
    // two transfers dispatched at the same instant: the id breaks the tie
    let tie_a = room.transfer(out(3, &[("THB", "100", 4)])).await;
    let tie_b = room.transfer(out(3, &[("USD", "100", 2)])).await;
    // another branch's transfer is never MAIN's
    let other = room
        .transfer(Seed {
            kind: "EXTERNAL_IN",
            from: B9,
            to: None,
            from_shift: None,
            to_shift: None,
            status: "RECEIVED",
            lines: &[("THB", "100", 9)],
            note: Some("B9"),
            at: t(5),
        })
        .await;

    let all = room.movements("branch=MAIN&limit=50").await;
    let all_ids = ids(&all);
    let expected: BTreeSet<String> = [
        format!("{newest}:THB"),
        format!("{oldest}:THB"),
        format!("{two_currencies}:THB"),
        format!("{two_currencies}:USD"),
        format!("{tie_a}:THB"),
        format!("{tie_b}:USD"),
    ]
    .into_iter()
    .collect();
    let listed: BTreeSet<String> = all_ids.iter().cloned().collect();
    assert_eq!(
        listed, expected,
        "every moved (transfer, currency) of MAIN, nothing of B9: {all}"
    );
    assert_eq!(all_ids.len(), expected.len(), "no row twice: {all}");
    assert!(!all_ids.iter().any(|id| id.starts_with(&other)), "{all}");
    assert_eq!(
        all_ids[0],
        format!("{newest}:THB"),
        "newest dispatched first: {all}"
    );
    assert_eq!(
        all_ids[5],
        format!("{oldest}:THB"),
        "oldest dispatched last: {all}"
    );
    let created: Vec<String> = list(&all)
        .iter()
        .map(|r| r["created"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert!(
        created.windows(2).all(|w| w[0] >= w[1]),
        "newest dispatched first throughout: {created:?}"
    );

    for limit in [1usize, 2, 4] {
        let mut walked: Vec<String> = Vec::new();
        let mut before: Option<String> = None;
        for _ in 0..20 {
            let query = match &before {
                Some(b) => format!("branch=MAIN&limit={limit}&before={b}"),
                None => format!("branch=MAIN&limit={limit}"),
            };
            let page = ids(&room.movements(&query).await);
            assert!(page.len() <= limit, "limit {limit}: {page:?}");
            if page.is_empty() {
                break;
            }
            before = page.last().cloned();
            walked.extend(page);
        }
        assert_eq!(
            walked, all_ids,
            "pages of {limit}, walked by `before`, are the whole list once, in order"
        );
    }
    room.finish().await;
}

// ── §3.2: the snapshot's movements ───────────────────────────────────────────

#[actix_web::test]
async fn the_snapshots_movements_list_the_moved_transfers_in_the_same_mapping() {
    let room = Room::open().await;
    let shift = room.open_shift(MAIN).await;
    let at = room.t0 + Duration::minutes(7);
    let tid = room
        .transfer(Seed {
            kind: "DRAWER_TO_SAFE",
            from: MAIN,
            to: None,
            from_shift: Some(&shift),
            to_shift: None,
            status: "DISPATCHED",
            lines: &[("THB", "100", 5)],
            note: Some("ส่งเงินเข้าเซฟ"),
            at,
        })
        .await;
    let draft = room
        .transfer(Seed {
            kind: "DRAWER_TO_SAFE",
            from: MAIN,
            to: None,
            from_shift: Some(&shift),
            to_shift: None,
            status: "DRAFT",
            lines: &[("USD", "100", 1)],
            note: None,
            at: at + Duration::minutes(1),
        })
        .await;

    let snapshot = room.snapshot(MAIN).await;
    let movements = &snapshot["stock"]["movements"];
    let mine = of_transfer(movements, &tid);
    assert_eq!(
        mine.len(),
        1,
        "one entry per (transfer, currency): {movements}"
    );
    let m = &mine[0];
    assert_eq!(m["id"], json!(format!("{tid}:THB")), "{m}");
    assert_eq!(m["shiftId"], json!(shift), "{m}");
    assert_eq!(m["created"], json!(iso(at)), "{m}");
    assert_eq!(m["staff"], json!(room.sender), "{m}");
    assert_eq!(m["currency"], json!("THB"), "{m}");
    assert_eq!(m["direction"], json!("OUT"), "{m}");
    assert_eq!(m["amount"], json!(500), "v2's amount is a number: {m}");
    assert_eq!(m["reason"], json!("ส่งเงินเข้าเซฟ"), "{m}");
    assert!(
        of_transfer(movements, &draft).is_empty(),
        "a DRAFT is not a move: {movements}"
    );
    room.finish().await;
}

// ── §3.2: shift_valuation takes the shift's transfers ────────────────────────

#[actix_web::test]
async fn the_valuation_lists_a_currency_the_shift_moved_and_nothing_it_did_not() {
    let room = Room::open().await;
    let shift = room.open_shift(MAIN).await;
    let t = |m: i64| room.t0 + Duration::minutes(m);
    // EUR: neither in the opening nor in the expected — only the shift's transfer names it
    room.transfer(Seed {
        kind: "SAFE_TO_DRAWER",
        from: MAIN,
        to: None,
        from_shift: None,
        to_shift: Some(&shift),
        status: "RECEIVED",
        lines: &[("EUR", "50", 2)],
        note: Some("เติม EUR"),
        at: t(1),
    })
    .await;
    // GBP: the branch's, not the shift's
    room.transfer(Seed {
        kind: "EXTERNAL_IN",
        from: MAIN,
        to: None,
        from_shift: None,
        to_shift: None,
        status: "RECEIVED",
        lines: &[("GBP", "50", 1)],
        note: None,
        at: t(2),
    })
    .await;
    // JPY: the shift's, but not moved yet
    room.transfer(Seed {
        kind: "SAFE_TO_DRAWER",
        from: MAIN,
        to: None,
        from_shift: None,
        to_shift: Some(&shift),
        status: "DRAFT",
        lines: &[("JPY", "1000", 1)],
        note: None,
        at: t(3),
    })
    .await;

    let answer = room.valuation(MAIN, &shift).await;
    let codes: Vec<String> = list(&answer["currencies"])
        .iter()
        .map(|c| c["currency"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(
        codes,
        vec!["THB", "EUR", "USD"],
        "THB first, then A–Z; EUR because the shift moved it; no GBP (not the shift's), no JPY (a DRAFT): {answer}"
    );
    room.finish().await;
}
