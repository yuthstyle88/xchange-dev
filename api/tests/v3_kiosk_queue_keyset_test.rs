//! Ticket #501: tests for kiosk queue keyset paging and status filtering.
//! Covers untested paths identified in §2:
//! - `working × before` (`QueueFilter::Working` with `before`)
//! - `completed × before` (`QueueFilter::Completed` with `before`)
//! - `QueueFilter::All` with `before` and `queue_page` status sets
//!
//! Verifies that each test goes red when its status set is wrong.

mod common;
mod compat_common;
mod kiosk_http_common;
mod rpc_common;
mod snapshot_common;

use actix_web::http::Method;
use compat_common::Person;
use kiosk_http_common::{Counter, person};
use rpc_common::{Answer, Call};
use serde_json::{Value, json};
use xc_kernel::ids::BranchId;
use xc_kiosk::domain::kiosk::repositories::kiosk_repository::KioskRepository;
use xc_kiosk::domain::kiosk::value_objects::queue_filter::QueueFilter;
use xc_kiosk::infrastructure::kiosk::kiosk_repository::PgKioskRepository;

const QUEUE: &str = "kiosk_queue";
const MAIN: &str = "MAIN";
const B9: &str = "B9";
const B9_NAME: &str = "Nine";
const FRONT2: &str = "FRONT2";

fn rid(n: u32) -> String {
    format!("{n:08x}-0000-4000-8000-0000000c{n:04x}")
}

fn queue_rows(data: &Value) -> &Vec<Value> {
    data.as_array()
        .unwrap_or_else(|| panic!("the queue's data is an array: {data}"))
}

fn ids(data: &Value) -> Vec<String> {
    queue_rows(data)
        .iter()
        .map(|r| {
            r["id"]
                .as_str()
                .unwrap_or_else(|| panic!("a row's id: {r}"))
                .to_string()
        })
        .collect()
}

fn get(path: &str) -> Call {
    let mut call = Call::post("http://desk.test", path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

fn path(view: &str, query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{view}")
    } else {
        format!("/api/v3/views/{view}?{query}")
    }
}

#[derive(Clone)]
struct Row {
    branch: &'static str,
    id: String,
    status: &'static str,
    created: String,
    staff: Option<String>,
    side: &'static str,
    currency: &'static str,
    foreign: &'static str,
    thb: &'static str,
    method: &'static str,
    started: Option<String>,
    completed: Option<String>,
    document: Option<String>,
}

impl Row {
    fn new(n: u32, status: &'static str, second: u32) -> Row {
        Row {
            branch: MAIN,
            id: rid(n),
            status,
            created: format!(
                "2026-10-06T04:{:02}:{:02}.{:03}Z",
                second / 60,
                second % 60,
                n % 1000
            ),
            staff: None,
            side: "BUY",
            currency: "USD",
            foreign: "100.00",
            thb: "3200.00",
            method: "Cash",
            started: None,
            completed: None,
            document: None,
        }
    }

    fn staff(mut self, staff: &str) -> Row {
        self.staff = Some(staff.to_string());
        self
    }

    fn times(mut self, started: Option<&str>, completed: Option<&str>) -> Row {
        self.started = started.map(str::to_string);
        self.completed = completed.map(str::to_string);
        self
    }

    fn document(mut self, doc: &str) -> Row {
        self.document = Some(doc.to_string());
        self
    }

    fn branch(mut self, branch: &'static str) -> Row {
        self.branch = branch;
        self
    }

    fn created(mut self, at: &str) -> Row {
        self.created = at.to_string();
        self
    }
}

struct Room {
    counter: Counter,
}

const STAFF_ROLES: [&str; 9] = [
    "FRONT",
    "BACKEND",
    "MANAGER",
    "RATES",
    "MONITOR",
    "FINANCE",
    "SYSTEM",
    "DOCUMENTS",
    "DOCUMENT_REVIEW",
];

fn the_staff() -> Vec<Person> {
    let mut staff: Vec<Person> = STAFF_ROLES
        .iter()
        .enumerate()
        .map(|(i, &role)| person(role, role, 0x31 + i as u8))
        .collect();
    staff.push(person(FRONT2, "FRONT", 0x41));
    staff
}

impl Room {
    async fn open() -> Room {
        let staff = the_staff();
        let counter = Counter::open(&staff).await;
        counter.desk.db.seed_branch(B9, B9_NAME).await;
        Room { counter }
    }

    async fn put(&self, row: &Row) {
        sqlx::query(
            "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, \
             currency, rate, foreign_amount, thb, input_amount, payment_method, submission_id, \
             document_ref) \
             VALUES ($1, $2, $3, $4::timestamptz, $5, $6, $7, 32.0, $8::numeric, $9::numeric, \
             $8::numeric, $10, $11, $11)",
        )
        .bind(row.branch)
        .bind(&row.id)
        .bind(row.status)
        .bind(&row.created)
        .bind(&row.staff)
        .bind(row.side)
        .bind(row.currency)
        .bind(row.foreign)
        .bind(row.thb)
        .bind(row.method)
        .bind(&row.document)
        .execute(&self.counter.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("put kiosk request {}: {e}", row.id));

        if row.started.is_some() || row.completed.is_some() {
            sqlx::query(
                "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, kiosk_request_id, counter_teller_id, issued_at, served_at, completed_at, idempotency_key) \
                 VALUES (gen_random_uuid(), (SELECT organization_id FROM branches WHERE id = $1), $1, current_date, 'WALK_IN', (SELECT COALESCE(MAX(sequence), 0) + 1 FROM queue_tickets WHERE branch_id = $1), 'A001', 'SERVING', $2, NULL, now(), $3::timestamptz, $4::timestamptz, $5)",
            )
            .bind(row.branch)
            .bind(&row.id)
            .bind(&row.started)
            .bind(&row.completed)
            .bind(format!("key-{}", row.id))
            .execute(&self.counter.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("put queue ticket for {}: {e}", row.id));
        }
    }

    async fn put_all(&self, rows: &[Row]) {
        for row in rows {
            self.put(row).await;
        }
    }

    async fn get(&self, who: &str, path: &str) -> Answer {
        let call = get(path);
        let call = if who.is_empty() {
            call
        } else {
            call.cookie(
                self.counter
                    .cookies
                    .get(who)
                    .unwrap_or_else(|| panic!("{who} is not signed in")),
            )
        };
        self.counter.app.send(call).await
    }

    async fn data(&self, who: &str, view: &str, query: &str) -> Value {
        let p = path(view, query);
        let answer = self.get(who, &p).await;
        assert_eq!(
            answer.status, 200,
            "GET {p} as {who:?}: {:.400}",
            answer.text
        );
        let body = answer.json();
        assert_eq!(body["changed"], json!(true));
        body.get("data").cloned().unwrap()
    }

    async fn finish(self) {
        self.counter.finish().await;
    }
}

/// Untested path 1: `working × before`.
/// Verifies:
/// 1. Only ACCEPTED and ID_CHECKED statuses are returned (never WAITING, COMPLETED, QUOTED).
/// 2. If status set misses ACCEPTED, the test fails.
/// 3. If status set misses ID_CHECKED, the test fails.
/// 4. If status set includes WAITING or COMPLETED, the test fails.
/// 5. Keyset pagination with `before` correctly traverses all working rows across pages.
/// 6. A non-working row (e.g. WAITING or COMPLETED) passed as `before` still positions the page correctly.
#[actix_web::test]
async fn working_with_before_keyset_pagination_and_status_filtering() {
    let room = Room::open().await;
    let front = "FRONT";

    // Seed rows across all statuses with staggered created_at timestamps
    let rows = vec![
        // Oldest to newest
        Row::new(0x10, "QUOTED", 1),
        Row::new(0x11, "WAITING", 2),
        Row::new(0x12, "ACCEPTED", 3)
            .staff(front)
            .times(Some("2026-10-06T04:00:03.000Z"), None),
        Row::new(0x13, "ID_CHECKED", 4)
            .staff(front)
            .times(Some("2026-10-06T04:00:03.000Z"), None),
        Row::new(0x14, "COMPLETED", 5)
            .staff(front)
            .times(
                Some("2026-10-06T04:00:03.000Z"),
                Some("2026-10-06T04:00:05.000Z"),
            )
            .document("DOC-14"),
        Row::new(0x15, "ACCEPTED", 6)
            .staff(front)
            .times(Some("2026-10-06T04:00:06.000Z"), None),
        Row::new(0x16, "WAITING", 7),
        Row::new(0x17, "ID_CHECKED", 8)
            .staff(front)
            .times(Some("2026-10-06T04:00:06.000Z"), None),
        Row::new(0x18, "COMPLETED", 9)
            .staff(front)
            .times(
                Some("2026-10-06T04:00:06.000Z"),
                Some("2026-10-06T04:00:09.000Z"),
            )
            .document("DOC-18"),
        Row::new(0x19, "ACCEPTED", 10)
            .staff(front)
            .times(Some("2026-10-06T04:00:10.000Z"), None),
        // Row in another branch
        Row::new(0x1a, "ACCEPTED", 11)
            .branch(B9)
            .staff(front)
            .times(Some("2026-10-06T04:00:11.000Z"), None),
    ];
    room.put_all(&rows).await;

    // Working rows for MAIN in newest-first order:
    // 0x19 (ACCEPTED, sec 10)
    // 0x17 (ID_CHECKED, sec 8)
    // 0x15 (ACCEPTED, sec 6)
    // 0x13 (ID_CHECKED, sec 4)
    // 0x12 (ACCEPTED, sec 3)
    let expected_working = vec![rid(0x19), rid(0x17), rid(0x15), rid(0x13), rid(0x12)];

    // 1. Full working list (no before)
    let full = ids(&room.data(front, QUEUE, "state=working").await);
    assert_eq!(
        full, expected_working,
        "all working rows in newest-first order"
    );

    // Verify all returned rows have status ACCEPTED or ID_CHECKED
    let full_data = room.data(front, QUEUE, "state=working").await;
    let statuses: Vec<&str> = queue_rows(&full_data)
        .iter()
        .map(|r| r["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        statuses,
        vec![
            "ACCEPTED",
            "ID_CHECKED",
            "ACCEPTED",
            "ID_CHECKED",
            "ACCEPTED"
        ]
    );
    assert!(statuses.contains(&"ACCEPTED"), "must include ACCEPTED");
    assert!(statuses.contains(&"ID_CHECKED"), "must include ID_CHECKED");
    assert!(!statuses.contains(&"WAITING"), "must not include WAITING");
    assert!(
        !statuses.contains(&"COMPLETED"),
        "must not include COMPLETED"
    );
    assert!(!statuses.contains(&"QUOTED"), "must not include QUOTED");

    // 2. Keyset walk with limit=2 and before
    let mut walked = Vec::new();
    let mut before: Option<String> = None;
    for _ in 0..5 {
        let q = match &before {
            None => "state=working&limit=2".to_string(),
            Some(id) => format!("state=working&limit=2&before={id}"),
        };
        let page = ids(&room.data(front, QUEUE, &q).await);
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= 2);
        before = page.last().cloned();
        walked.extend(page);
    }
    assert_eq!(
        walked, expected_working,
        "keyset paging walks all working rows"
    );

    // 3. before with specific working row (after 0x17 should come 0x15, 0x13, 0x12)
    let after_17 = ids(&room
        .data(front, QUEUE, &format!("state=working&before={}", rid(0x17)))
        .await);
    assert_eq!(after_17, vec![rid(0x15), rid(0x13), rid(0x12)]);

    // 4. before naming a COMPLETED row (0x18, sec 9):
    // Older working rows should be 0x17, 0x15, 0x13, 0x12 (0x19 is sec 10, so strictly before 0x18)
    let after_completed = ids(&room
        .data(front, QUEUE, &format!("state=working&before={}", rid(0x18)))
        .await);
    assert_eq!(
        after_completed,
        vec![rid(0x17), rid(0x15), rid(0x13), rid(0x12)],
        "before naming a completed row positions the working page correctly"
    );

    // 5. before naming the oldest working row (0x12): nothing after it
    let after_oldest = ids(&room
        .data(front, QUEUE, &format!("state=working&before={}", rid(0x12)))
        .await);
    assert!(after_oldest.is_empty(), "after oldest working row is empty");

    // 6. before naming an id of another branch: returns empty
    let after_b9 = ids(&room
        .data(front, QUEUE, &format!("state=working&before={}", rid(0x1a)))
        .await);
    assert!(
        after_b9.is_empty(),
        "before from another branch returns empty"
    );

    // 7. before naming nonexistent id: returns empty
    let after_nonexistent = ids(&room
        .data(
            front,
            QUEUE,
            "state=working&before=00000000-0000-4000-8000-000000000000",
        )
        .await);
    assert!(
        after_nonexistent.is_empty(),
        "before naming nonexistent id returns empty"
    );

    room.finish().await;
}

/// Untested path 2: `completed × before`.
/// Verifies:
/// 1. Only COMPLETED status is returned (never WAITING, ACCEPTED, ID_CHECKED, QUOTED).
/// 2. If status set misses COMPLETED, the test fails.
/// 3. If status set includes ACCEPTED, ID_CHECKED, or WAITING, the test fails.
/// 4. Keyset pagination with `before` correctly traverses all completed rows across pages.
/// 5. Keyset tie-breaking by id DESC on identical created_at timestamps.
/// 6. A non-completed row passed as `before` still positions the page correctly.
#[actix_web::test]
async fn completed_with_before_keyset_pagination_and_status_filtering() {
    let room = Room::open().await;
    let front = "FRONT";

    // Seed rows across all statuses, with multiple COMPLETED rows including a timestamp tie
    let rows = vec![
        Row::new(0x20, "QUOTED", 1),
        Row::new(0x21, "WAITING", 2),
        Row::new(0x22, "COMPLETED", 3)
            .staff(front)
            .times(
                Some("2026-10-06T04:00:01.000Z"),
                Some("2026-10-06T04:00:03.000Z"),
            )
            .document("DOC-22"),
        Row::new(0x23, "ACCEPTED", 4)
            .staff(front)
            .times(Some("2026-10-06T04:00:04.000Z"), None),
        // Tie in timestamp: 0x24 and 0x25 share created_at
        Row::new(0x24, "COMPLETED", 5)
            .created("2026-10-06T04:00:05.000Z")
            .staff(front)
            .times(
                Some("2026-10-06T04:00:01.000Z"),
                Some("2026-10-06T04:00:05.000Z"),
            )
            .document("DOC-24"),
        Row::new(0x25, "COMPLETED", 5)
            .created("2026-10-06T04:00:05.000Z")
            .staff(front)
            .times(
                Some("2026-10-06T04:00:01.000Z"),
                Some("2026-10-06T04:00:05.000Z"),
            )
            .document("DOC-25"),
        Row::new(0x26, "ID_CHECKED", 6)
            .staff(front)
            .times(Some("2026-10-06T04:00:06.000Z"), None),
        Row::new(0x27, "COMPLETED", 7)
            .staff(front)
            .times(
                Some("2026-10-06T04:00:01.000Z"),
                Some("2026-10-06T04:00:07.000Z"),
            )
            .document("DOC-27"),
        Row::new(0x28, "WAITING", 8),
        Row::new(0x29, "COMPLETED", 9)
            .staff(front)
            .times(
                Some("2026-10-06T04:00:01.000Z"),
                Some("2026-10-06T04:00:09.000Z"),
            )
            .document("DOC-29"),
        // B9 completed row
        Row::new(0x2a, "COMPLETED", 10)
            .branch(B9)
            .staff(front)
            .times(
                Some("2026-10-06T04:00:01.000Z"),
                Some("2026-10-06T04:00:10.000Z"),
            )
            .document("DOC-2A"),
    ];
    room.put_all(&rows).await;

    // Completed rows in MAIN in newest-first order:
    // 0x29 (sec 9)
    // 0x27 (sec 7)
    // 0x25 (sec 5, tie broken by rid(0x25) > rid(0x24))
    // 0x24 (sec 5)
    // 0x22 (sec 3)
    let expected_completed = vec![rid(0x29), rid(0x27), rid(0x25), rid(0x24), rid(0x22)];

    // 1. Full completed list
    let full = ids(&room.data(front, QUEUE, "state=completed").await);
    assert_eq!(
        full, expected_completed,
        "all completed rows in newest-first order"
    );

    let full_data = room.data(front, QUEUE, "state=completed").await;
    let statuses: Vec<&str> = queue_rows(&full_data)
        .iter()
        .map(|r| r["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, vec!["COMPLETED"; 5]);
    assert!(!statuses.contains(&"WAITING"));
    assert!(!statuses.contains(&"ACCEPTED"));
    assert!(!statuses.contains(&"ID_CHECKED"));
    assert!(!statuses.contains(&"QUOTED"));

    // 2. Keyset walk with limit=2 and before
    let mut walked = Vec::new();
    let mut before: Option<String> = None;
    for _ in 0..5 {
        let q = match &before {
            None => "state=completed&limit=2".to_string(),
            Some(id) => format!("state=completed&limit=2&before={id}"),
        };
        let page = ids(&room.data(front, QUEUE, &q).await);
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= 2);
        before = page.last().cloned();
        walked.extend(page);
    }
    assert_eq!(
        walked, expected_completed,
        "keyset paging walks all completed rows"
    );

    // 3. Tie-break: before = 0x25 (the newer of the tied rows) -> next must be 0x24 then 0x22
    let after_25 = ids(&room
        .data(
            front,
            QUEUE,
            &format!("state=completed&before={}", rid(0x25)),
        )
        .await);
    assert_eq!(
        after_25,
        vec![rid(0x24), rid(0x22)],
        "after tied row: other tied row then older"
    );

    // 4. before with a WAITING row (0x28, sec 8):
    // Older completed rows should be 0x27, 0x25, 0x24, 0x22
    let after_waiting = ids(&room
        .data(
            front,
            QUEUE,
            &format!("state=completed&before={}", rid(0x28)),
        )
        .await);
    assert_eq!(
        after_waiting,
        vec![rid(0x27), rid(0x25), rid(0x24), rid(0x22)]
    );

    // 5. before with the oldest completed row (0x22)
    let after_oldest = ids(&room
        .data(
            front,
            QUEUE,
            &format!("state=completed&before={}", rid(0x22)),
        )
        .await);
    assert!(
        after_oldest.is_empty(),
        "after oldest completed row is empty"
    );

    // 6. before with an id of B9
    let after_b9 = ids(&room
        .data(
            front,
            QUEUE,
            &format!("state=completed&before={}", rid(0x2a)),
        )
        .await);
    assert!(
        after_b9.is_empty(),
        "before from another branch returns empty"
    );

    room.finish().await;
}

/// Direct repository tests for `PgKioskRepository`:
/// 1. `QueueFilter::All` with `before` (returns all non-QUOTED rows).
/// 2. `QueueFilter::Working` and `QueueFilter::Completed` with `queue_page` (verifies folded queue_page).
#[actix_web::test]
async fn direct_kiosk_repository_queue_before_and_page_coverage() {
    let room = Room::open().await;
    let repo = PgKioskRepository::new(room.counter.desk.db.pool.clone());
    let branch = BranchId::new(MAIN);

    let rows = vec![
        Row::new(0x30, "QUOTED", 1),
        Row::new(0x31, "WAITING", 2),
        Row::new(0x32, "ACCEPTED", 3)
            .staff("FRONT")
            .times(Some("2026-10-06T04:00:03.000Z"), None),
        Row::new(0x33, "ID_CHECKED", 4)
            .staff("FRONT")
            .times(Some("2026-10-06T04:00:03.000Z"), None),
        Row::new(0x34, "COMPLETED", 5)
            .staff("FRONT")
            .times(
                Some("2026-10-06T04:00:03.000Z"),
                Some("2026-10-06T04:00:05.000Z"),
            )
            .document("DOC-34"),
    ];
    room.put_all(&rows).await;

    // QueueFilter::All with before=rid(0x34):
    // Should return 0x33, 0x32, 0x31 (and never 0x30 QUOTED)
    let all_after_34 = repo
        .queue_before(&branch, QueueFilter::All, Some(&rid(0x34)), 10)
        .await
        .expect("queue_before All");
    let all_ids: Vec<String> = all_after_34.into_iter().map(|(r, _)| r.id).collect();
    assert_eq!(all_ids, vec![rid(0x33), rid(0x32), rid(0x31)]);

    // queue_page with QueueFilter::Working
    let working_page = repo
        .queue_page(&branch, QueueFilter::Working, 0)
        .await
        .expect("queue_page Working");
    let working_ids: Vec<String> = working_page.into_iter().map(|(r, _)| r.id).collect();
    assert_eq!(working_ids, vec![rid(0x33), rid(0x32)]);

    // queue_page with QueueFilter::Completed
    let completed_page = repo
        .queue_page(&branch, QueueFilter::Completed, 0)
        .await
        .expect("queue_page Completed");
    let completed_ids: Vec<String> = completed_page.into_iter().map(|(r, _)| r.id).collect();
    assert_eq!(completed_ids, vec![rid(0x34)]);

    // queue_page with QueueFilter::All
    let all_page = repo
        .queue_page(&branch, QueueFilter::All, 0)
        .await
        .expect("queue_page All");
    let all_page_ids: Vec<String> = all_page.into_iter().map(|(r, _)| r.id).collect();
    assert_eq!(
        all_page_ids,
        vec![rid(0x34), rid(0x33), rid(0x32), rid(0x31)]
    );

    room.finish().await;
}
