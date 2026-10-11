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
//!
//! Re-expected for RE-7 #604 by the ticket author (claude/07d3765f). RE-7 removes
//! `RateService::save_rate` and drops `rates` (`docs/design/blueprint/rates.md` §8 migration 6).
//! The design's §9 row "`rates_test.rs`, `api/tests/v3_admin_rates_view_test.rs` | the direct save
//! | RE-7 / RE-5 | publications" retires every test here that pinned the direct save, and
//! `RateService::rate_in`, which read the newest `rates` row (no reader reads `rates` since RE-4;
//! the counter rate is the current publication's line, §4.8). Retired:
//! `every_rate_scenario_answers_as_v2_did` (the fixture's `saveRate` scenarios; the fixture is
//! kept as v2's record), `a_saved_rate_carries_the_time_it_was_given_cut_to_the_millisecond`,
//! `the_newest_row_is_the_last_one_written_not_the_latest_clock`,
//! `the_reference_rows_of_the_currency_are_stored_with_the_rate`,
//! `a_repeated_request_does_not_ask_the_source_again`,
//! `a_request_refused_on_its_own_content_never_reaches_the_source`,
//! `when_the_source_fails_nothing_is_saved_and_its_sentence_is_the_answer`,
//! `a_save_raises_the_branch_version_and_a_refusal_does_not`,
//! `the_branch_is_not_locked_while_the_source_is_being_asked`,
//! `a_rate_saved_while_the_source_was_slow_makes_the_slow_save_stale`,
//! `of_many_screens_saving_over_the_same_row_exactly_one_wins`,
//! `the_same_request_sent_many_times_at_once_writes_one_row`,
//! `a_request_id_spent_in_another_branch_is_not_a_repeat_here`,
//! `rate_in_reads_the_counter_rate_inside_the_callers_transaction` and
//! `a_refused_save_for_an_unknown_branch_leaves_no_trace_of_it`. Rates are set only by
//! publication now: draft → submit → publish is pinned in `rate_draft_flow_test.rs`, the readers in
//! `published_rates_test.rs`, partner and the AUTO base in `rates_re7_partner_test.rs`.
//! Kept: the fixture's shape, the register as `current` lists it (every currency, no figures:
//! what `current` answers after RE-7), the migration's seed (less the `rates` count, a table RE-7
//! drops) and the refusal for a branch that does not exist.

mod common;

use common::*;

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
    // RE-7 #604: `rates` is dropped (migration 6), so it is not counted here.
    assert_eq!(db.count("abandoned_requests").await, 0);
    db.finish().await;
}

// ── Part 2: what the recording cannot show ──────────────────────────────────

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
