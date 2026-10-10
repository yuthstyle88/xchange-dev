//! QU-A1 (#556 design row QU-1) — the queue's migration M1 against a real PostgreSQL: the five
//! tables of design §4.1, one `branch_service_status` row per existing branch, and the unique and
//! CHECK rules that keep a number unique per branch, day and line.
//! Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/queue-tickets.md` §4.1 (tables), §8 M1 (create only; one status
//! row per branch with `queue_open = false`, `active_counters = 0`, NULL estimate), §9.1 row QU-1.
//! Constraint names are not pinned: each rule is pinned by the row it refuses (SQLSTATE `23514`
//! CHECK, `23503` foreign key, `23505` unique).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;

use queue_common::*;

const CHECK: &str = "23514";
const FOREIGN_KEY: &str = "23503";
const UNIQUE: &str = "23505";

const TABLES: [&str; 5] = [
    "queue_sequences",
    "queue_tickets",
    "queue_events",
    "queue_counters",
    "branch_service_status",
];

/// A ticket row with every NOT NULL column; `extra` names more columns and `values` their values.
fn ticket(id: &str, seq: i32, service_type: &str, key: &str, extra: &str, values: &str) -> String {
    let display = format!("{}{seq:03}", &service_type[..1]);
    format!(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, \
             sequence, display_number, status, issued_at, idempotency_key{extra}) \
         VALUES ('{id}', '{ORG_1}', 'MAIN', DATE '2026-11-02', '{service_type}', {seq}, \
             '{display}', 'WAITING', now(), '{key}'{values})"
    )
}

const T1: &str = "00000000-0000-0000-0000-0000000000a1";
const T2: &str = "00000000-0000-0000-0000-0000000000a2";
const T3: &str = "00000000-0000-0000-0000-0000000000a3";

#[tokio::test]
async fn m1_applies_on_a_fresh_database_and_creates_the_five_tables() {
    let db = TestDb::start().await;
    for table in TABLES {
        let org = db
            .text(&format!(
                "SELECT data_type::text || ':' || is_nullable::text FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = '{table}' \
                   AND column_name = 'organization_id'"
            ))
            .await;
        assert_eq!(
            org.as_deref(),
            Some("uuid:NO"),
            "{table}.organization_id is uuid NOT NULL (§4.1 header)"
        );
        let created = db
            .count(&format!(
                "SELECT count(*) FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = '{table}' \
                   AND column_name = 'created_at'"
            ))
            .await;
        assert_eq!(created, 1, "{table} has created_at (§4.1 header)");
    }
    assert_eq!(
        db.count("SELECT count(*) FROM branch_service_status").await,
        1,
        "a fresh database has one branch (MAIN) and one status row"
    );
    db.finish().await;
}

#[tokio::test]
async fn m1_on_a_deployed_database_gives_every_branch_one_closed_status_row() {
    // The test database on do-sgp1 already ran every later migration; M1 is applied after them.
    let db = TestDb::empty().await;
    db.migrate_all_but_m1().await;
    db.add_branch("B2", "Asia/Tokyo").await;
    db.migrate_all()
        .await
        .expect("M1 applies after every other migration already ran");
    assert_eq!(
        db.count("SELECT count(*) FROM branch_service_status").await,
        2,
        "one status row per existing branch (§8 M1 item 1)"
    );
    assert_eq!(
        db.count(
            "SELECT count(*) FROM branch_service_status s JOIN branches b ON b.id = s.branch_id \
             WHERE s.organization_id = b.organization_id AND s.queue_open = false \
               AND s.active_counters = 0 AND s.queue_version = 0 AND s.estimate_seconds IS NULL \
               AND s.estimate_low_seconds IS NULL AND s.estimate_high_seconds IS NULL \
               AND s.estimate_as_of IS NULL"
        )
        .await,
        2,
        "each status row: the branch's organisation, queue closed, no counter, unknown estimate"
    );
    for table in [
        "queue_sequences",
        "queue_tickets",
        "queue_events",
        "queue_counters",
    ] {
        assert_eq!(
            db.count(&format!("SELECT count(*) FROM {table}")).await,
            0,
            "{table} starts empty"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn a_number_is_unique_per_branch_date_and_line() {
    let db = TestDb::start().await;
    assert_eq!(
        db.outcome(&ticket(T1, 1, "WALK_IN", "k1", "", "")).await,
        "ok"
    );
    assert_eq!(
        db.outcome(&ticket(T2, 1, "WALK_IN", "k2", "", "")).await,
        UNIQUE,
        "a second W001 on the same branch and day is refused"
    );
    db.add_booking("MAIN", "BK-1").await;
    assert_eq!(
        db.outcome(&ticket(T3, 1, "BOOKING", "k3", ", booking_id", ", 'BK-1'"))
            .await,
        "ok",
        "B001 counts apart from W001 (owner Q-A1)"
    );
    db.finish().await;
}

#[tokio::test]
async fn an_idempotency_key_is_used_once_per_branch() {
    let db = TestDb::start().await;
    assert_eq!(
        db.outcome(&ticket(T1, 1, "WALK_IN", "same", "", "")).await,
        "ok"
    );
    assert_eq!(
        db.outcome(&ticket(T2, 2, "WALK_IN", "same", "", "")).await,
        UNIQUE,
        "UNIQUE (branch_id, idempotency_key)"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_kiosk_request_has_at_most_one_ticket() {
    let db = TestDb::start().await;
    db.add_kiosk_request("MAIN", "KR-1").await;
    let with_request = |id: &str, seq: i32, key: &str| {
        ticket(id, seq, "WALK_IN", key, ", kiosk_request_id", ", 'KR-1'")
    };
    assert_eq!(db.outcome(&with_request(T1, 1, "k1")).await, "ok");
    assert_eq!(
        db.outcome(&with_request(T2, 2, "k2")).await,
        UNIQUE,
        "a second ticket for one kiosk request is refused"
    );
    assert_eq!(
        db.outcome(&ticket(
            T3,
            3,
            "WALK_IN",
            "k3",
            ", kiosk_request_id",
            ", 'KR-NONE'"
        ))
        .await,
        FOREIGN_KEY,
        "the kiosk request must exist in the branch"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_booking_has_at_most_one_active_ticket() {
    let db = TestDb::start().await;
    db.add_booking("MAIN", "BK-1").await;
    let booked =
        |id: &str, seq: i32, key: &str| ticket(id, seq, "BOOKING", key, ", booking_id", ", 'BK-1'");
    assert_eq!(db.outcome(&booked(T1, 1, "k1")).await, "ok");
    assert_eq!(
        db.outcome(&booked(T2, 2, "k2")).await,
        UNIQUE,
        "a second active ticket for one booking is refused"
    );
    db.exec(&format!(
        "UPDATE queue_tickets SET status = 'COMPLETED' WHERE id = '{T1}'"
    ))
    .await;
    assert_eq!(
        db.outcome(&booked(T2, 2, "k2")).await,
        "ok",
        "once the first ticket has ended, the booking may have a new active one"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_booking_line_and_booking_id_go_together() {
    let db = TestDb::start().await;
    db.add_booking("MAIN", "BK-1").await;
    assert_eq!(
        db.outcome(&ticket(T1, 1, "BOOKING", "k1", "", "")).await,
        CHECK,
        "a BOOKING ticket without a booking is refused"
    );
    assert_eq!(
        db.outcome(&ticket(T2, 1, "WALK_IN", "k2", ", booking_id", ", 'BK-1'"))
            .await,
        CHECK,
        "a WALK_IN ticket with a booking is refused"
    );
    assert_eq!(
        db.outcome(&ticket(
            T3,
            1,
            "BOOKING",
            "k3",
            ", booking_id",
            ", 'BK-NONE'"
        ))
        .await,
        FOREIGN_KEY,
        "the booking must exist"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_ticket_checks_refuse_values_outside_the_design() {
    let db = TestDb::start().await;
    assert_eq!(
        db.outcome(&ticket(T1, 0, "WALK_IN", "k1", "", "")).await,
        CHECK,
        "sequence > 0"
    );
    assert_eq!(
        db.outcome(&ticket(T1, 1, "PRIORITY", "k1", "", "")).await,
        CHECK,
        "service_type is WALK_IN or BOOKING"
    );
    assert_eq!(
        db.outcome(&ticket(T1, 1, "WALK_IN", "k1", "", "").replace("'WAITING'", "'PAUSED'"))
            .await,
        CHECK,
        "status is one of the six of §4.2"
    );
    assert_eq!(
        db.outcome(&ticket(
            T1,
            1,
            "WALK_IN",
            "k1",
            ", counter_teller_id",
            ", '00000000-0000-0000-0000-00000000ffff'"
        ))
        .await,
        FOREIGN_KEY,
        "counter_teller_id is a users.id"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_status_row_keeps_the_estimate_and_its_range_together() {
    let db = TestDb::start().await;
    let set = |est: &str, low: &str, high: &str| {
        format!(
            "UPDATE branch_service_status SET estimate_seconds = {est}, \
             estimate_low_seconds = {low}, estimate_high_seconds = {high} WHERE branch_id = 'MAIN'"
        )
    };
    assert_eq!(db.outcome(&set("300", "210", "390")).await, "ok");
    assert_eq!(
        db.outcome(&set("300", "NULL", "390")).await,
        CHECK,
        "the three are NULL together (Q-A6)"
    );
    assert_eq!(
        db.outcome(&set("300", "310", "390")).await,
        CHECK,
        "low <= estimate"
    );
    assert_eq!(
        db.outcome(&set("300", "210", "290")).await,
        CHECK,
        "estimate <= high"
    );
    assert_eq!(db.outcome(&set("NULL", "NULL", "NULL")).await, "ok");
    assert_eq!(
        db.outcome("UPDATE branch_service_status SET active_counters = -1")
            .await,
        CHECK,
        "active_counters >= 0"
    );
    db.finish().await;
}

#[tokio::test]
async fn events_and_counters_refuse_unknown_kinds_and_states() {
    let db = TestDb::start().await;
    db.exec(&ticket(T1, 1, "WALK_IN", "k1", "", "")).await;
    let event = |kind: &str| {
        format!(
            "INSERT INTO queue_events (id, organization_id, ticket_id, status, kind, at) \
             VALUES (gen_random_uuid(), '{ORG_1}', '{T1}', 'WAITING', '{kind}', now())"
        )
    };
    assert_eq!(db.outcome(&event("ISSUE")).await, "ok");
    assert_eq!(
        db.outcome(&event("PAUSE")).await,
        CHECK,
        "kind is one of seven"
    );
    let teller = db.add_user("Teller").await;
    let counter = |status: &str| {
        format!(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             VALUES ('{ORG_1}', 'MAIN', '{teller}', '{status}')"
        )
    };
    assert_eq!(
        db.outcome(&counter("IDLE")).await,
        CHECK,
        "ACTIVE or PAUSED"
    );
    assert_eq!(db.outcome(&counter("ACTIVE")).await, "ok");
    assert_eq!(
        db.outcome(&counter("PAUSED")).await,
        UNIQUE,
        "one counter per teller per branch"
    );
    db.finish().await;
}
