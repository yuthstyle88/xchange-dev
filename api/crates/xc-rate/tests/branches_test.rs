//! Written before the implementation (ticket T5a). Do not edit, delete or weaken.
//!
//! Specification of `xc_rate::application::branch::service::BranchService`: the
//! shop's branches (design §4 `branches`).
//!
//! Part 1 replays what the v2 engine answered (`tests/fixtures/v2_rate_cases.json`).
//! Part 2 is what a single-threaded recording cannot show.
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.

mod common;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use common::*;
use pos_error::AppError;
use tokio::sync::Barrier;

fn create(name: &str) -> CreateBranch {
    CreateBranch {
        request_id: request_id(),
        name: name.to_owned(),
        from_branch: branch(MAIN),
    }
}

// ── Part 1: the v2 recording ────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_branch_scenario_answers_as_v2_did() {
    play_all(|s| kind(s) == "createBranch").await;
}

// ── Part 2: what the recording cannot show ──────────────────────────────────

#[tokio::test]
async fn a_fresh_database_has_the_main_branch_only() {
    let db = TestDb::start().await;
    let list = db.branches().list().await.expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id.as_str(), "MAIN");
    assert_eq!(list[0].name, "XC-B");
    assert_eq!(list[0].request_id, None);
    db.finish().await;
}

#[tokio::test]
async fn a_created_branch_records_who_and_when_and_is_named_after_its_request() {
    let db = TestDb::start().await;
    let cmd = create("Silom");
    let now = at(0);

    let made = db
        .branches()
        .create(cmd.clone(), ACTOR, now)
        .await
        .expect("create");

    assert_eq!(made.id.as_str(), format!("B{}", cmd.request_id));
    assert_eq!(made.name, "Silom");
    assert_eq!(made.request_id.as_deref(), Some(cmd.request_id.as_str()));
    let (created_at, created_by): (DateTime<Utc>, String) =
        sqlx::query_as("SELECT created_at, created_by FROM branches WHERE id = $1")
            .bind(made.id.as_str())
            .fetch_one(&db.pool)
            .await
            .expect("read the branch row");
    assert_eq!(
        created_at,
        millis(now),
        "the time it was given, cut to the millisecond"
    );
    assert_eq!(created_by, ACTOR);
    db.finish().await;
}

#[tokio::test]
async fn branches_are_listed_main_first_then_in_the_order_they_were_created() {
    let db = TestDb::start().await;
    let branches = db.branches();
    // Names and request ids chosen so that neither sorts in creation order.
    for (n, name) in ["Zulu", "Alpha", "Mike"].into_iter().enumerate() {
        let cmd = CreateBranch {
            request_id: format!("req-{}-aaaaaaaaaaaa", 9 - n),
            name: name.to_owned(),
            from_branch: branch(MAIN),
        };
        branches
            .create(cmd, ACTOR, at(n as i64))
            .await
            .expect("create");
    }
    let names: Vec<String> = branches
        .list()
        .await
        .expect("list")
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert_eq!(names, ["XC-B", "Zulu", "Alpha", "Mike"]);
    db.finish().await;
}

#[tokio::test]
async fn require_returns_the_branch_or_refuses() {
    let db = TestDb::start().await;
    let branches = db.branches();
    let made = branches
        .create(create("Silom"), ACTOR, at(0))
        .await
        .expect("create");

    assert_eq!(
        branches.require(&branch(MAIN)).await.expect("MAIN").name,
        "XC-B"
    );
    assert_eq!(
        branches.require(&made.id).await.expect("the new branch"),
        made
    );
    assert_eq!(
        refusal(branches.require(&branch("BNOPE")).await),
        NO_SUCH_BRANCH
    );
    assert_eq!(
        refusal(branches.require(&branch("main")).await),
        NO_SUCH_BRANCH,
        "branch ids are exact"
    );
    assert_eq!(refusal(branches.require(&branch("")).await), NO_SUCH_BRANCH);
    db.finish().await;
}

#[tokio::test]
async fn a_new_branch_takes_rates_of_its_own_at_once() {
    let db = TestDb::start().await;
    let made = db
        .branches()
        .create(create("Silom"), ACTOR, at(0))
        .await
        .expect("create");
    let rates = db.rates();
    let cmd = SaveRate {
        request_id: request_id(),
        code: "USD".to_owned(),
        buy: "31".to_owned(),
        sell: "33".to_owned(),
        partner_name: String::new(),
        partner_cost: None,
        previous_updated: String::new(),
    };
    rates
        .save_rate(&made.id, cmd, ACTOR, at(1))
        .await
        .expect("a rate in the new branch");
    assert_eq!(rates.history(&made.id).await.expect("history").len(), 1);
    assert!(
        rates
            .history(&branch(MAIN))
            .await
            .expect("history")
            .is_empty()
    );
    db.finish().await;
}

#[tokio::test]
async fn a_refused_creation_writes_nothing() {
    let db = TestDb::start().await;
    let branches = db.branches();
    assert_eq!(
        refusal(branches.create(create("xc-b"), ACTOR, at(0)).await),
        NAME_TAKEN
    );
    assert_eq!(
        refusal(branches.create(create(""), ACTOR, at(0)).await),
        BAD_TEXT
    );
    let mut from_nowhere = create("Silom");
    from_nowhere.from_branch = branch("BNOPE");
    assert_eq!(
        refusal(branches.create(from_nowhere, ACTOR, at(0)).await),
        NO_SUCH_BRANCH
    );
    assert_eq!(db.count("branches").await, 1);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn of_many_screens_creating_the_same_name_exactly_one_wins() {
    const SCREENS: usize = 8;
    let db = TestDb::start().await;
    // They stand on different branches: the name must still be unique in the shop.
    db.seed_branch("BOTHER", "Other").await;
    let branches = Arc::new(db.branches());
    let barrier = Arc::new(Barrier::new(SCREENS));
    let mut tasks = Vec::new();
    for n in 0..SCREENS {
        let (branches, barrier) = (branches.clone(), barrier.clone());
        tasks.push(tokio::spawn(async move {
            let mut cmd = create(if n % 2 == 0 { "Silom" } else { "SILOM" });
            if n % 4 >= 2 {
                cmd.from_branch = branch("BOTHER");
            }
            barrier.wait().await;
            branches.create(cmd, ACTOR, at(n as i64)).await
        }));
    }
    let mut won = 0;
    for task in tasks {
        match within(MUST_FINISH, "a concurrent creation", task)
            .await
            .unwrap()
        {
            Ok(_) => won += 1,
            Err(AppError::Domain(sentence)) => assert_eq!(sentence, NAME_TAKEN),
            Err(other) => panic!("a losing screen must be told the name is taken, got {other:?}"),
        }
    }
    assert_eq!(won, 1);
    assert_eq!(db.count("branches").await, 3);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_twentieth_place_goes_to_exactly_one_of_many_screens() {
    const SCREENS: usize = 6;
    let db = TestDb::start().await;
    for n in 0..18 {
        db.seed_branch(&format!("BSEED{n}"), &format!("Seeded {n}"))
            .await;
    }
    assert_eq!(db.count("branches").await, 19);
    let branches = Arc::new(db.branches());
    let barrier = Arc::new(Barrier::new(SCREENS));
    let mut tasks = Vec::new();
    for n in 0..SCREENS {
        let (branches, barrier) = (branches.clone(), barrier.clone());
        tasks.push(tokio::spawn(async move {
            let mut cmd = create(&format!("Racer {n}"));
            cmd.from_branch = branch(if n % 2 == 0 { MAIN } else { "BSEED0" });
            barrier.wait().await;
            branches.create(cmd, ACTOR, at(n as i64)).await
        }));
    }
    let mut won = 0;
    for task in tasks {
        match within(MUST_FINISH, "a concurrent creation", task)
            .await
            .unwrap()
        {
            Ok(_) => won += 1,
            Err(AppError::Domain(sentence)) => assert_eq!(sentence, TOO_MANY_BRANCHES),
            Err(other) => panic!("a losing screen must be told the shop is full, got {other:?}"),
        }
    }
    assert_eq!(won, 1, "twenty branches at most, however many ask at once");
    assert_eq!(db.count("branches").await, 20);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_creation_sent_many_times_at_once_makes_one_branch() {
    const COPIES: usize = 6;
    let db = TestDb::start().await;
    let branches = Arc::new(db.branches());
    let cmd = create("Silom");
    let barrier = Arc::new(Barrier::new(COPIES));
    let mut tasks = Vec::new();
    for n in 0..COPIES {
        let (branches, barrier, cmd) = (branches.clone(), barrier.clone(), cmd.clone());
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            branches.create(cmd, ACTOR, at(n as i64)).await
        }));
    }
    for task in tasks {
        let made = within(MUST_FINISH, "a copy of the request", task)
            .await
            .unwrap()
            .expect("every copy of one request is answered with the branch");
        assert_eq!(made.id.as_str(), format!("B{}", cmd.request_id));
    }
    assert_eq!(db.count("branches").await, 2);
    db.finish().await;
}
