//! Written before the implementation (ticket T5a). Do not edit, delete or weaken.
//!
//! Specification of `xc_rate::application::rate::service::RateService`: the
//! counter rates of a branch (design §3 "Save rate", §4 `rates`, `currencies`).
//!
//! Part 1 replays what the v2 engine answered, step by step
//! (`tests/fixtures/v2_rate_cases.json`, recorded by `api/tools/v2-fixtures/gen_rates.mjs`).
//! Part 2 is what a recording of a single-threaded engine cannot show: the
//! reference-rate source, time, concurrency and the branch lock.
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.

mod common;

use std::sync::Arc;

use common::*;
use pos_error::AppError;
use tokio::sync::Barrier;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate};

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").expect("USD is in the register")
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).expect("a valid rate")
}

/// A well-formed first save of `code` in a branch that has no row for it.
fn first(code: &str, buy: &str, sell: &str) -> SaveRate {
    SaveRate {
        request_id: request_id(),
        code: code.to_owned(),
        buy: buy.to_owned(),
        sell: sell.to_owned(),
        partner_name: String::new(),
        partner_cost: None,
        previous_updated: String::new(),
    }
}

async fn current_of(rates: &RateService, branch_id: &BranchId, code: &str) -> CounterRate {
    rates
        .current(branch_id)
        .await
        .expect("current")
        .into_iter()
        .find(|r| r.code.as_str() == code)
        .unwrap_or_else(|| panic!("current() has no entry for {code}"))
}

// ── Part 1: the v2 recording ────────────────────────────────────────────────

#[test]
fn the_fixture_is_the_one_this_file_was_written_for() {
    let fixture = fixture();
    assert_eq!(fixture.scenarios.len(), 93);
    assert_eq!(
        fixture
            .scenarios
            .iter()
            .map(|s| s.steps.len())
            .sum::<usize>(),
        156
    );
    assert_eq!(fixture.currencies.len(), 152);
    let mut known = vec![
        NO_SUCH_BRANCH,
        BAD_TEXT,
        BAD_REQUEST_ID,
        BAD_NUMBER,
        TOO_MANY_DECIMALS,
        SELL_BELOW_BUY,
        ABANDONED,
        PARTNER_PAIR,
        REQUEST_CHANGED,
        EDITED_ELSEWHERE,
        NAME_TAKEN,
        TOO_MANY_BRANCHES,
    ];
    known.sort_unstable();
    assert_eq!(
        fixture.sentences, known,
        "the sentences v2 used are the twelve constants"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_rate_scenario_answers_as_v2_did() {
    play_all(|s| kind(s) == "saveRate").await;
}

#[tokio::test]
async fn a_new_branch_lists_every_currency_of_the_register_without_a_rate() {
    let db = TestDb::start().await;
    let fixture = fixture();
    let list = db.rates().current(&branch(MAIN)).await.expect("current");

    let got: Vec<(String, String, Option<String>)> = list
        .iter()
        .map(|r| (r.code.as_str().to_owned(), r.name.clone(), r.symbol.clone()))
        .collect();
    let want: Vec<(String, String, Option<String>)> = fixture
        .currencies
        .iter()
        .map(|c| (c.code.clone(), c.name.clone(), c.symbol.clone()))
        .collect();
    assert_eq!(
        got, want,
        "code, Thai name and symbol, in v2's order (the register without THB)"
    );

    for r in &list {
        assert!(
            r.buy.is_none()
                && r.sell.is_none()
                && r.updated_at.is_none()
                && r.request_id.is_none()
                && r.partner_name.is_empty()
                && r.partner_cost.is_none()
                && r.reference.is_none(),
            "{} must be empty before its first rate: {r:?}",
            r.code
        );
    }
    assert!(
        db.rates()
            .history(&branch(MAIN))
            .await
            .expect("history")
            .is_empty()
    );
    db.finish().await;
}

#[tokio::test]
async fn the_migration_seeds_the_register_and_the_main_branch() {
    let db = TestDb::start().await;
    assert_eq!(
        db.count("currencies").await,
        153,
        "the 152 currencies with a rate row, and THB"
    );
    let thb: i64 = sqlx::query_scalar("SELECT count(*) FROM currencies WHERE code = 'THB'")
        .fetch_one(&db.pool)
        .await
        .expect("read currencies");
    assert_eq!(thb, 1);
    let main: String = sqlx::query_scalar("SELECT name FROM branches WHERE id = 'MAIN'")
        .fetch_one(&db.pool)
        .await
        .expect("the main branch is seeded");
    assert_eq!(main, fixture().main_branch.name);
    assert_eq!(db.count("branches").await, 1);
    assert_eq!(db.count("rates").await, 0);
    assert_eq!(db.count("abandoned_requests").await, 0);
    db.finish().await;
}

// ── Part 2: what the recording cannot show ──────────────────────────────────

#[tokio::test]
async fn a_saved_rate_carries_the_time_it_was_given_cut_to_the_millisecond() {
    let db = TestDb::start().await;
    let rates = db.rates();
    let now = at(0);
    assert_ne!(now, millis(now), "the test's clock has microseconds");

    rates
        .save_rate(&branch(MAIN), first("USD", "32.1", "32.6"), ACTOR, now)
        .await
        .expect("save");

    let row = current_of(&rates, &branch(MAIN), "USD").await;
    assert_eq!(row.updated_at, Some(millis(now)));
    assert_eq!(row.buy, Some(rate("32.1")));
    assert_eq!(row.sell, Some(rate("32.6")));
    let history = rates.history(&branch(MAIN)).await.expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].updated_at, millis(now));
    assert_eq!(history[0].updated_by, ACTOR);
    assert_eq!(history[0].code, usd());

    // What the screen sends back is that time as JavaScript prints it.
    let mut next = first("USD", "32.2", "32.7");
    next.previous_updated = iso(millis(now));
    rates
        .save_rate(&branch(MAIN), next, ACTOR, at(1))
        .await
        .expect("the printed time names the row");
    db.finish().await;
}

#[tokio::test]
async fn the_newest_row_is_the_last_one_written_not_the_latest_clock() {
    let db = TestDb::start().await;
    let rates = db.rates();
    let main = branch(MAIN);

    rates
        .save_rate(&main, first("USD", "32.1", "32.6"), ACTOR, at(100))
        .await
        .expect("first");
    // A second row stamped EARLIER than the first (a clock that stepped back).
    let mut second = first("USD", "32.2", "32.7");
    second.previous_updated = iso(millis(at(100)));
    rates
        .save_rate(&main, second, ACTOR, at(50))
        .await
        .expect("second");
    // A third stamped with the very same instant as the second.
    let mut third = first("USD", "32.3", "32.8");
    third.previous_updated = iso(millis(at(50)));
    rates
        .save_rate(&main, third.clone(), ACTOR, at(50))
        .await
        .expect("third");

    let row = current_of(&rates, &main, "USD").await;
    assert_eq!(
        row.buy,
        Some(rate("32.3")),
        "current is the row written last"
    );
    assert_eq!(row.request_id.as_deref(), Some(third.request_id.as_str()));
    let buys: Vec<String> = rates
        .history(&main)
        .await
        .expect("history")
        .iter()
        .map(|h| h.buy.to_string())
        .collect();
    assert_eq!(
        buys,
        ["32.3000", "32.2000", "32.1000"],
        "history is newest first, by order of writing"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_reference_rows_of_the_currency_are_stored_with_the_rate() {
    let db = TestDb::start().await;
    let source = FakeSource::answering();
    let rates = db.rates_with(source.clone());
    let main = branch(MAIN);

    rates
        .save_rate(&main, first("USD", "32.1", "32.6"), ACTOR, at(0))
        .await
        .expect("USD");
    rates
        .save_rate(&main, first("JPY", "0.21", "0.22"), ACTOR, at(1))
        .await
        .expect("JPY");
    // A currency the source has no row for still stores the answer, with no rows.
    rates
        .save_rate(&main, first("ZWG", "1", "2"), ACTOR, at(2))
        .await
        .expect("ZWG");

    assert_eq!(source.calls(), 3, "one call per new rate row");
    for code in ["USD", "JPY", "ZWG"] {
        let row = current_of(&rates, &main, code).await;
        assert_eq!(
            row.reference,
            Some(FakeSource::stored_for(code)),
            "reference stored for {code}"
        );
    }
    let history = rates.history(&main).await.expect("history");
    assert_eq!(history[0].reference, Some(FakeSource::stored_for("ZWG")));
    assert_eq!(history[2].reference, Some(FakeSource::stored_for("USD")));
    db.finish().await;
}

#[tokio::test]
async fn a_repeated_request_does_not_ask_the_source_again() {
    let db = TestDb::start().await;
    let source = FakeSource::answering();
    let rates = db.rates_with(source.clone());
    let cmd = first("USD", "32.1", "32.6");

    rates
        .save_rate(&branch(MAIN), cmd.clone(), ACTOR, at(0))
        .await
        .expect("first time");
    rates
        .save_rate(&branch(MAIN), cmd.clone(), ACTOR, at(9))
        .await
        .expect("the same request again");
    let mut changed = cmd.clone();
    changed.buy = "32.2".to_owned();
    assert_eq!(
        refusal(rates.save_rate(&branch(MAIN), changed, ACTOR, at(10)).await),
        REQUEST_CHANGED
    );

    assert_eq!(source.calls(), 1);
    assert_eq!(db.count("rates").await, 1);
    let row = current_of(&rates, &branch(MAIN), "USD").await;
    assert_eq!(
        row.updated_at,
        Some(millis(at(0))),
        "a repeat does not restamp the row"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_request_refused_on_its_own_content_never_reaches_the_source() {
    let db = TestDb::start().await;
    let source = FakeSource::answering();
    let rates = db.rates_with(source.clone());
    let with = |f: fn(&mut SaveRate)| {
        let mut cmd = first("USD", "32.1", "32.6");
        f(&mut cmd);
        cmd
    };
    let cases: Vec<(SaveRate, &str)> = vec![
        (with(|c| c.request_id = "short".to_owned()), BAD_REQUEST_ID),
        (with(|c| c.code = String::new()), BAD_TEXT),
        (with(|c| c.buy = "0".to_owned()), BAD_NUMBER),
        (
            with(|c| c.sell = "32.600000001".to_owned()),
            TOO_MANY_DECIMALS,
        ),
        (with(|c| c.sell = "32".to_owned()), SELL_BELOW_BUY),
        (with(|c| c.code = "THB".to_owned()), SELL_BELOW_BUY),
        (with(|c| c.partner_name = "P".to_owned()), PARTNER_PAIR),
        (
            with(|c| c.partner_cost = Some("32".to_owned())),
            PARTNER_PAIR,
        ),
        (with(|c| c.partner_name = "x".repeat(101)), BAD_TEXT),
    ];
    for (cmd, sentence) in cases {
        let shown = format!("{cmd:?}");
        assert_eq!(
            refusal(rates.save_rate(&branch(MAIN), cmd, ACTOR, at(0)).await),
            sentence,
            "{shown}"
        );
    }
    assert_eq!(
        refusal(
            rates
                .save_rate(&branch("BNOPE"), first("USD", "1", "2"), ACTOR, at(0))
                .await
        ),
        NO_SUCH_BRANCH
    );
    assert_eq!(
        source.calls(),
        0,
        "no refused request may cost a call to the reference source"
    );
    assert_eq!(db.count("rates").await, 0);
    db.finish().await;
}

#[tokio::test]
async fn when_the_source_fails_nothing_is_saved_and_its_sentence_is_the_answer() {
    let db = TestDb::start().await;
    let source = FakeSource::failing();
    let rates = db.rates_with(source.clone());
    let version = db
        .branch_tx()
        .version(&branch(MAIN))
        .await
        .expect("version");

    let answer = rates
        .save_rate(&branch(MAIN), first("USD", "32.1", "32.6"), ACTOR, at(0))
        .await;

    assert_eq!(refusal(answer), SOURCE_DOWN);
    assert_eq!(source.calls(), 1);
    assert_eq!(db.count("rates").await, 0);
    assert_eq!(
        db.branch_tx()
            .version(&branch(MAIN))
            .await
            .expect("version"),
        version
    );
    db.finish().await;
}

#[tokio::test]
async fn a_save_raises_the_branch_version_and_a_refusal_does_not() {
    let db = TestDb::start().await;
    let rates = db.rates();
    db.seed_branch("BOTHER", "Other").await;
    let tx = db.branch_tx();
    let main = branch(MAIN);
    let other = branch("BOTHER");
    let before = (
        tx.version(&main).await.unwrap(),
        tx.version(&other).await.unwrap(),
    );

    rates
        .save_rate(&main, first("USD", "32.1", "32.6"), ACTOR, at(0))
        .await
        .expect("save");
    let after_save = tx.version(&main).await.unwrap();
    assert!(
        after_save > before.0,
        "a written rate must raise the version of its branch"
    );
    assert_eq!(
        tx.version(&other).await.unwrap(),
        before.1,
        "and of no other branch"
    );

    // previous_updated is empty although USD now has a row: edited elsewhere.
    assert_eq!(
        refusal(
            rates
                .save_rate(&main, first("USD", "32.2", "32.7"), ACTOR, at(1))
                .await
        ),
        EDITED_ELSEWHERE
    );
    assert_eq!(
        refusal(
            rates
                .save_rate(&main, first("USD", "9", "1"), ACTOR, at(2))
                .await
        ),
        SELL_BELOW_BUY
    );
    assert_eq!(
        tx.version(&main).await.unwrap(),
        after_save,
        "a refusal leaves the version alone"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_branch_is_not_locked_while_the_source_is_being_asked() {
    let db = TestDb::start().await;
    let (source, entered, release) = FakeSource::holding();
    let rates = Arc::new(db.rates_with(source.clone()));
    let main = branch(MAIN);

    let slow = {
        let (rates, main) = (rates.clone(), main.clone());
        tokio::spawn(async move {
            rates
                .save_rate(&main, first("USD", "32.1", "32.6"), ACTOR, at(0))
                .await
        })
    };
    within(
        MUST_FINISH,
        "the save reaches the source",
        entered.acquire(),
    )
    .await
    .unwrap()
    .forget();

    // The save is now waiting for the source. Another writer of the same branch gets through.
    within(
        MUST_FINISH,
        "a second writer of the branch while the source is slow",
        async {
            db.branch_tx()
                .run(&main, |_tx| Box::pin(async { Ok(()) }))
                .await
                .expect("an empty write")
        },
    )
    .await;
    assert_eq!(
        db.count("rates").await,
        0,
        "nothing is written before the source answered"
    );

    release.add_permits(1);
    within(MUST_FINISH, "the slow save finishes", slow)
        .await
        .unwrap()
        .expect("the slow save");
    assert_eq!(db.count("rates").await, 1);
    db.finish().await;
}

#[tokio::test]
async fn a_rate_saved_while_the_source_was_slow_makes_the_slow_save_stale() {
    let db = TestDb::start().await;
    let (source, entered, release) = FakeSource::holding();
    let slow_rates = Arc::new(db.rates_with(source));
    let quick_rates = db.rates();
    let main = branch(MAIN);

    let slow = {
        let (rates, main) = (slow_rates.clone(), main.clone());
        tokio::spawn(async move {
            rates
                .save_rate(&main, first("USD", "30", "31"), ACTOR, at(0))
                .await
        })
    };
    within(
        MUST_FINISH,
        "the slow save reaches the source",
        entered.acquire(),
    )
    .await
    .unwrap()
    .forget();
    within(
        MUST_FINISH,
        "a quick save of the same branch while the source is slow",
        quick_rates.save_rate(&main, first("USD", "32.1", "32.6"), ACTOR, at(1)),
    )
    .await
    .expect("the quick save");
    release.add_permits(1);

    let answer = within(MUST_FINISH, "the slow save finishes", slow)
        .await
        .unwrap();
    assert_eq!(
        refusal(answer),
        EDITED_ELSEWHERE,
        "previous_updated is judged when the row is written"
    );
    assert_eq!(
        current_of(&quick_rates, &main, "USD").await.buy,
        Some(rate("32.1"))
    );
    assert_eq!(db.count("rates").await, 1);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn of_many_screens_saving_over_the_same_row_exactly_one_wins() {
    const SCREENS: usize = 8;
    let db = TestDb::start().await;
    let rates = Arc::new(db.rates());
    let main = branch(MAIN);
    rates
        .save_rate(&main, first("USD", "32.1", "32.6"), ACTOR, at(0))
        .await
        .expect("the row they all saw");
    let seen = iso(millis(at(0)));

    let barrier = Arc::new(Barrier::new(SCREENS));
    let mut tasks = Vec::new();
    for n in 0..SCREENS {
        let (rates, main, barrier, seen) =
            (rates.clone(), main.clone(), barrier.clone(), seen.clone());
        tasks.push(tokio::spawn(async move {
            let mut cmd = first("USD", &format!("33.{n}"), "34");
            cmd.previous_updated = seen;
            barrier.wait().await;
            rates.save_rate(&main, cmd, ACTOR, at(10 + n as i64)).await
        }));
    }
    let mut won = 0;
    for task in tasks {
        match within(MUST_FINISH, "a concurrent save", task)
            .await
            .unwrap()
        {
            Ok(()) => won += 1,
            Err(AppError::Domain(sentence)) => assert_eq!(sentence, EDITED_ELSEWHERE),
            Err(other) => {
                panic!("a losing screen must be told the rate was edited elsewhere, got {other:?}")
            }
        }
    }
    assert_eq!(
        won, 1,
        "exactly one of {SCREENS} screens may replace the row"
    );
    assert_eq!(db.count("rates").await, 2);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_request_sent_many_times_at_once_writes_one_row() {
    const COPIES: usize = 8;
    let db = TestDb::start().await;
    let source = FakeSource::answering();
    let rates = Arc::new(db.rates_with(source));
    let main = branch(MAIN);
    let cmd = first("EUR", "37.5", "38");

    let barrier = Arc::new(Barrier::new(COPIES));
    let mut tasks = Vec::new();
    for n in 0..COPIES {
        let (rates, main, barrier, cmd) =
            (rates.clone(), main.clone(), barrier.clone(), cmd.clone());
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            rates.save_rate(&main, cmd, ACTOR, at(n as i64)).await
        }));
    }
    for task in tasks {
        within(MUST_FINISH, "a copy of the request", task)
            .await
            .unwrap()
            .expect("every copy of one request is answered as saved");
    }
    assert_eq!(db.count("rates").await, 1);
    db.finish().await;
}

#[tokio::test]
async fn a_request_id_spent_in_another_branch_is_not_a_repeat_here() {
    let db = TestDb::start().await;
    db.seed_branch("BOTHER", "Other").await;
    let rates = db.rates();
    let cmd = first("USD", "32.1", "32.6");

    rates
        .save_rate(&branch(MAIN), cmd.clone(), ACTOR, at(0))
        .await
        .expect("in the main branch");
    let mut other = cmd.clone();
    other.buy = "31".to_owned();
    rates
        .save_rate(&branch("BOTHER"), other, ACTOR, at(1))
        .await
        .expect(
            "v2 keeps request ids per branch: the same id with other data is a new request there",
        );

    assert_eq!(
        current_of(&rates, &branch(MAIN), "USD").await.buy,
        Some(rate("32.1"))
    );
    assert_eq!(
        current_of(&rates, &branch("BOTHER"), "USD").await.buy,
        Some(rate("31"))
    );
    db.finish().await;
}

#[tokio::test]
async fn rate_in_reads_the_counter_rate_inside_the_callers_transaction() {
    let db = TestDb::start().await;
    let rates = db.rates();
    let main = branch(MAIN);
    db.seed_branch("BOTHER", "Other").await;
    rates
        .save_rate(&main, first("USD", "32.1", "32.6"), ACTOR, at(0))
        .await
        .expect("save");
    let mut newer = first("USD", "32.2", "32.7");
    newer.previous_updated = iso(millis(at(0)));
    rates
        .save_rate(&main, newer, ACTOR, at(1))
        .await
        .expect("newer");

    let read = |branch_id: BranchId, code: &'static str| {
        let tx = db.branch_tx();
        async move {
            let code = CurrencyCode::parse(code).unwrap();
            let inner = branch_id.clone();
            tx.run(&branch_id, move |tx| {
                Box::pin(async move { RateService::rate_in(tx, &inner, code).await })
            })
            .await
            .expect("rate_in")
        }
    };
    assert_eq!(
        read(main.clone(), "USD").await,
        Some((rate("32.2"), rate("32.7"))),
        "(buy, sell) of the newest row"
    );
    assert_eq!(
        read(main.clone(), "EUR").await,
        None,
        "a currency with no row has no counter rate"
    );
    assert_eq!(
        read(branch("BOTHER"), "USD").await,
        None,
        "another branch's rate is not this branch's"
    );
    db.finish().await;
}

#[tokio::test]
async fn reading_the_rates_of_a_branch_that_does_not_exist_is_refused() {
    let db = TestDb::start().await;
    let rates = db.rates();
    assert_eq!(
        refusal(rates.current(&branch("BNOPE")).await),
        NO_SUCH_BRANCH
    );
    assert_eq!(
        refusal(rates.history(&branch("BNOPE")).await),
        NO_SUCH_BRANCH
    );
    assert_eq!(
        db.count("branch_versions").await,
        0,
        "a read creates no lock row"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_refused_save_for_an_unknown_branch_leaves_no_trace_of_it() {
    let db = TestDb::start().await;
    let rates = db.rates();
    assert_eq!(
        refusal(
            rates
                .save_rate(&branch("BNOPE"), first("USD", "1", "2"), ACTOR, at(0))
                .await
        ),
        NO_SUCH_BRANCH
    );
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM branch_versions WHERE branch_id = 'BNOPE'")
            .fetch_one(&db.pool)
            .await
            .expect("read branch_versions");
    assert_eq!(
        rows, 0,
        "the branch must be checked before BranchTx::run, which would create its lock row"
    );
    db.finish().await;
}
