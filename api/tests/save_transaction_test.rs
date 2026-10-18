//! Specification tests for T8b: saving an exchange.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! `SaveTransactionWorkflow::save(branch, command, actor_id, now)` judges the
//! form, checks the customer, the open shift, the counter rate and the stock,
//! takes the next document number, moves the stock, keeps the exchange and its
//! receipt, and writes the customer — all of it or none of it.
//!
//! `fixtures/save/v2_save_cases.json` was recorded by playing 28 scenarios (107
//! saves) against the v2 engine (`api/tools/v2-fixtures/gen_save.mjs`); CI-4
//! (#651) retired one (27 scenarios, 99 saves remain) and holds five reserves to
//! decided customer differences (save_common::SAME_SCENARIO_CUSTOMER); the
//! first test replays them. The others are what the engine could not be asked:
//! the attached files, which v2 checks before its engine runs; failures in the
//! middle; saves at the same moment.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose
//! name ends in `_test`). Without it they FAIL; none skips.

mod save_common;

use std::collections::{BTreeMap, BTreeSet};

use save_common::*;
use xc_kernel::money::{Rate, Side};

// ------------------------------------------------------------ the v2 fixture

/// One scenario on a fresh shop. Returns every place where the answer or what
/// was left behind differs from v2.
async fn play(fx: &Fixture, scenario: &Scenario) -> Vec<String> {
    let shop = Shop::start().await;
    let mut wrong = Vec::new();
    // label -> branch id; (branch label, alias) -> key of the uploaded file
    let mut branches: BTreeMap<String, String> =
        BTreeMap::from([(MAIN.to_owned(), MAIN.to_owned())]);
    let mut keys: BTreeMap<(String, String), String> = BTreeMap::new();
    // a branch nobody created, for the scenario that needs one
    branches.insert("GONE".to_owned(), "XC-B99".to_owned());
    // CI-4 (#651): step -> the customer v3 reserved; recorded customer id -> the earlier customer it stands for
    let mut reserved: BTreeMap<usize, String> = BTreeMap::new();
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    let mut written: Vec<WrittenSave> = Vec::new();
    // CA-10 (#626, the BE controller's ruling): a save moves stock on the saver's own OPEN shift and one teller
    // holds one OPEN shift, so where v2's one actor saved outside MAIN, that branch's save is made (and its files
    // uploaded) by the branch's own opener (`Shop::teller_of`); at MAIN the owner is the opener. Every expected
    // figure stays the same.

    for (i, step) in scenario.steps.iter().enumerate() {
        let op = step["op"].as_str().expect("a step has an op");
        let here = format!("'{}' step {} ({op})", scenario.name, i + 1);
        let label = step["branch"]
            .as_str()
            .expect("a step has a branch")
            .to_owned();
        let s = |key: &str| {
            step[key]
                .as_str()
                .unwrap_or_else(|| panic!("{here}: {key}"))
                .to_owned()
        };
        match op {
            "createBranch" => {
                shop.seed_branch(&s("id"), &s("name")).await;
                branches.insert(label, s("id"));
            }
            "rate" => {
                shop.set_rate(&branches[&label], &s("code"), &s("buy"), &s("sell"))
                    .await
            }
            "openShift" => {
                let opening: Vec<(String, String)> = step["opening"]
                    .as_object()
                    .expect("an opening")
                    .iter()
                    .map(|(c, a)| (c.clone(), a.as_str().expect("text").to_owned()))
                    .collect();
                let pairs: Vec<(&str, &str)> = opening
                    .iter()
                    .map(|(c, a)| (c.as_str(), a.as_str()))
                    .collect();
                shop.open_shift(&branches[&label], &pairs).await;
            }
            "closeShift" => shop.close_shift(&branches[&label]).await,
            "reserve" => {
                let who = ReserveCustomer {
                    name: s("name"),
                    passport_no: s("passportNo"),
                    nationality: s("nationality"),
                    birth_date: s("birthDate"),
                    passport_expiry: s("passportExpiry"),
                    mode: s("mode"),
                };
                if let Some(same) = same_scenario_customer(&scenario.name, i + 1) {
                    // CI-4 (#651): a decided customer difference, held strictly
                    let got = shop
                        .customers
                        .reserve(&branch(&branches[&label]), who, at(-30))
                        .await
                        .expect("reserve a customer");
                    let label = format!("{here} ({})", same.reason);
                    hold_same_scenario_customer(
                        &got.customer_id,
                        got.existing,
                        &reserved[&same.same_as],
                        &label,
                    );
                    aliases.insert(s("customerId"), got.customer_id.clone());
                    reserved.insert(i + 1, got.customer_id);
                    continue;
                }
                let id = shop.reserve(&branches[&label], who).await;
                assert_eq!(
                    id,
                    s("customerId"),
                    "{here}: the customer number (ticket T6a's)"
                );
                reserved.insert(i + 1, id);
            }
            "save" => {
                let save: FxSave =
                    serde_json::from_value(step.clone()).unwrap_or_else(|e| panic!("{here}: {e}"));
                let here = format!("{here} '{}'", save.note);
                let branch_id = branches[&label].clone();
                let exists = label != "GONE";
                let saver = if exists && branch_id != MAIN {
                    shop.teller_of(&branch_id)
                        .await
                        .unwrap_or_else(|| panic!("{here}: the opener of {branch_id}"))
                } else {
                    fx.actor.clone()
                };
                // the files the form names, uploaded by the owner into this branch
                let key_of = |alias: &Option<String>| -> Option<(String, String)> {
                    alias.as_ref().map(|a| (label.clone(), a.clone()))
                };
                let mut uploaded = Vec::new();
                for wanted in [key_of(&save.passport), key_of(&save.slip)] {
                    let Some(k) = wanted else {
                        uploaded.push(None);
                        continue;
                    };
                    if !keys.contains_key(&k) {
                        let file = &fx.files[&k.1];
                        let key = if exists {
                            shop.upload(&branch_id, &saver, &file.mime, &file.bytes())
                                .await
                        } else {
                            // nothing can be uploaded into a branch that is not there: the
                            // save is asked without files, as v2's engine was
                            String::new()
                        };
                        keys.insert(k.clone(), key);
                    }
                    uploaded.push(Some(keys[&k].clone()).filter(|k| !k.is_empty()));
                }
                let mut form = save.form.clone();
                let aliased = form
                    .get("customerId")
                    .and_then(|v| v.as_str())
                    .and_then(|c| aliases.get(c))
                    .cloned();
                if let Some(actual) = &aliased {
                    form.insert(
                        "customerId".to_owned(),
                        serde_json::Value::String(actual.clone()),
                    );
                }
                let cmd = command(&form, uploaded[0].clone(), uploaded[1].clone());
                let got = shop
                    .workflow
                    .save(&branch(&branch_id), cmd, &saver, instant(&save.at))
                    .await;

                match (&save.v2_error, &save.saved, &got) {
                    (Some(want), _, Err(pos_error::AppError::Domain(sentence)))
                        if sentence == want => {}
                    (None, Some(v2), Ok(saved)) => {
                        let key = |alias: &str| {
                            format!("/files/{}", keys[&(label.clone(), alias.to_owned())])
                        };
                        let want = SavedTransaction {
                            mode: if v2.mode == "DEMO" {
                                Mode::Demo
                            } else {
                                Mode::Live
                            },
                            document_ref: v2.document_ref.clone(),
                            passport_url: key(&v2.passport),
                            slip_url: v2.slip.as_deref().map(key).unwrap_or_default(),
                            // the address is the SHA-256 of the receipt: equal addresses
                            // mean the receipt is v2's byte for byte
                            receipt_url: v2.receipt_url.clone(),
                        };
                        // CI-4 (#651): the receipt names the aliased customer, so its address is held to the
                        // hash of the receipt v3 stored, not to v2's
                        let want = if aliased.is_some() {
                            let key = saved.receipt_url.trim_start_matches("/files/");
                            let bytes = std::fs::read(shop.dir.join(key))
                                .unwrap_or_else(|e| panic!("{here}: the receipt {key}: {e}"));
                            hold_receipt_hash(&saved.receipt_url, &bytes, &here);
                            SavedTransaction {
                                receipt_url: saved.receipt_url.clone(),
                                ..want
                            }
                        } else {
                            want
                        };
                        if saved != &want {
                            wrong.push(format!("{here}: v2 answered {want:?}, got {saved:?}"));
                        }
                        let text = |k: &str| {
                            form.get(k)
                                .and_then(|v| v.as_str())
                                .filter(|v| !v.is_empty())
                                .map(str::to_owned)
                        };
                        written.push(WrittenSave {
                            customer: text("customerId").unwrap_or_default(),
                            document_ref: v2.document_ref.clone(),
                            name: text("name").unwrap_or_default(),
                            birth_date: text("birthDate"),
                            passport_expiry: text("passportExpiry"),
                        });
                    }
                    (want, _, got) => {
                        wrong.push(format!("{here}: v2 answered {want:?}, got {got:?}"))
                    }
                }

                let Some(after) = &save.after else { continue };
                let balances = shop.balances(&branch_id).await;
                if balances != after.balances {
                    wrong.push(format!(
                        "{here}: balances are {balances:?}, v2 has {:?}",
                        after.balances
                    ));
                }
                let live = shop.refs(&branch_id, Mode::Live).await;
                let demo = shop.refs(&branch_id, Mode::Demo).await;
                if live != after.live || demo != after.demo {
                    wrong.push(format!(
                        "{here}: exchanges are {live:?} / {demo:?}, v2 has {:?} / {:?}",
                        after.live, after.demo
                    ));
                }
                let counters = shop.counters().await;
                if counters != after.counters {
                    wrong.push(format!(
                        "{here}: counters are {counters:?}, v2 has {:?}",
                        after.counters
                    ));
                }
                // every live exchange is in a shift, and two are in the same shift
                // exactly when v2 has them in the same shift; a demo exchange is in none
                let mut ours: BTreeMap<String, Option<String>> = BTreeMap::new();
                for t in shop
                    .transactions
                    .list(&branch(&branch_id), Mode::Live)
                    .await
                    .unwrap()
                {
                    ours.insert(t.document_ref, t.shift_id);
                }
                for (a, shift_a) in &after.shift_of {
                    for (b, shift_b) in &after.shift_of {
                        if (shift_a == shift_b) != (ours.get(a) == ours.get(b))
                            || ours.get(a).is_none_or(|s| s.is_none())
                        {
                            wrong.push(format!(
                                "{here}: shifts of {a} and {b}: {:?} and {:?}",
                                ours.get(a),
                                ours.get(b)
                            ));
                        }
                    }
                }
                for t in shop
                    .transactions
                    .list(&branch(&branch_id), Mode::Demo)
                    .await
                    .unwrap()
                {
                    if t.shift_id.is_some() {
                        wrong.push(format!(
                            "{here}: demo exchange {} is in a shift",
                            t.document_ref
                        ));
                    }
                }
                // CI-4 (#651): an aliased customer is folded into its earlier customer
                let expected: Vec<FxCustomer> = if aliases.is_empty() {
                    after.customers.clone()
                } else {
                    fold_aliased_customers(&after.customers, &aliases, &written)
                };
                let mut seen = Vec::new();
                for c in &expected {
                    let stored = shop
                        .customers
                        .find(&branch(&branch_id), &c.id)
                        .await
                        .unwrap();
                    // CI-4 (#651): the birth date and expiry live in the encrypted passport identity (#554 §4.1).
                    let passport = passport_payload(&shop.pool, &c.id).await;
                    let text_of = |k: &str| passport[k].as_str().map(str::to_owned);
                    seen.push(stored.map(|s| FxCustomer {
                        id: s.id,
                        name: s.name,
                        birth_date: text_of("birthDate"),
                        passport_expiry: text_of("expiry"),
                        last_document_ref: s.last_document_ref,
                        mode: s.mode.as_str().to_owned(),
                    }));
                }
                let want: Vec<Option<&FxCustomer>> = expected.iter().map(Some).collect();
                if seen.iter().map(Option::as_ref).collect::<Vec<_>>() != want {
                    wrong.push(format!(
                        "{here}: customers are {seen:?}, v2 has {expected:?} (as folded)",
                    ));
                }
            }
            other => panic!("{here}: the fixture has a step this test does not know: {other}"),
        }
    }
    if wrong.is_empty() {
        shop.finish().await;
    }
    wrong
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_scenario_ends_as_v2_ended() {
    let fx = fixture();
    assert_eq!(fx.actor, OWNER);
    assert_eq!(
        (
            fx.counts["scenarios"],
            fx.counts["saves"],
            fx.counts["saved"],
            fx.counts["refused"]
        ),
        // CI-4 (#651): "the customer id must be that passport in that branch and mode" (8 saves: 1 saved,
        // 7 refused) is retired; its rule is gone (#554 K1, owner 2026-10-09: DEMO shares LIVE).
        (27, 99, 54, 45),
        "v2_save_cases.json was cut short"
    );
    assert_eq!(fx.scenarios.len(), 27);
    assert_eq!(fx.sentences.len(), 11, "{:?}", fx.sentences);

    let mut wrong = Vec::new();
    for scenario in &fx.scenarios {
        wrong.extend(play(&fx, scenario).await);
    }
    assert!(
        wrong.is_empty(),
        "{} differences from v2:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

// ------------------------------------------------------------ what is kept

#[tokio::test]
async fn a_saved_exchange_can_be_read_back_whole() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let slip = shop.upload(MAIN, OWNER, "application/pdf", PDF).await;
    let shift = shop.open_shift_id(MAIN).await;

    let mut cmd = form("whole-0001-aaaaaaaaaa", &customer, &passport);
    cmd.customer.name = "  Somchai J. Jaidee ".to_owned();
    cmd.customer.birth_date = "1990-01-31".to_owned();
    cmd.side = "SELL".to_owned();
    cmd.rate = "33".to_owned();
    cmd.amount = "250.5".to_owned();
    cmd.fee = "10".to_owned();
    cmd.bank_fee = "1.5".to_owned();
    cmd.method = "Bank Transfer".to_owned();
    cmd.payment_ref = "TRX-1".to_owned();
    cmd.slip = Some(slip.clone());
    cmd.notes = " a note ".to_owned();
    cmd.paid_at = "2026-10-06T12:30:00+07:00".to_owned();
    cmd.reference_rate = Some("32.95".to_owned());
    cmd.value_date = "2026-11-01".to_owned();
    let saved = shop.save(MAIN, cmd, at(0)).await.expect("a save");

    assert_eq!(saved.mode, Mode::Live);
    assert_eq!(saved.document_ref, "XC-20261006-000001");
    assert_eq!(saved.passport_url, format!("/files/{passport}"));
    assert_eq!(saved.slip_url, format!("/files/{slip}"));
    let receipt_key = saved
        .receipt_url
        .strip_prefix("/files/")
        .expect("a file address")
        .to_owned();

    let stored = shop
        .transactions
        .find(&branch(MAIN), "XC-20261006-000001")
        .await
        .unwrap()
        .expect("the exchange is kept");
    assert_eq!(
        stored,
        Transaction {
            document_ref: "XC-20261006-000001".to_owned(),
            branch_id: branch(MAIN),
            mode: Mode::Live,
            shift_id: Some(shift),
            submission_id: "whole-0001-aaaaaaaaaa".to_owned(),
            customer_id: customer.clone(),
            customer_name: "Somchai J. Jaidee".to_owned(),
            nationality: "THA".to_owned(),
            side: Side::Sell,
            currency: "USD".to_owned(),
            amount: money("250.5"),
            rate: Rate::parse("33").unwrap(),
            reference_rate: Some(Rate::parse("32.95").unwrap()),
            principal: money("8266.5"),
            fee: money("10"),
            bank_fee: money("1.5"),
            received: money("8276.5"),
            paid: money("0"),
            method: xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod::BankTransfer,
            payment_ref: "TRX-1".to_owned(),
            account: "Cash drawer 1".to_owned(),
            staff: "EMP-001".to_owned(),
            booking: "WALK-IN".to_owned(),
            notes: "a note".to_owned(),
            value_date: chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
            paid_at: instant("2026-10-06T05:30:00Z"),
            passport_key: passport.clone(),
            slip_key: Some(slip),
            receipt_key: receipt_key.clone(),
            // the time the save was given, cut to milliseconds; who saved
            recorded_at: millis(at(0)),
            recorded_by: OWNER.to_owned(),
        }
    );
    // found in its own branch only
    shop.seed_branch("XC-B01", "Second").await;
    assert_eq!(
        shop.transactions
            .find(&branch("XC-B01"), "XC-20261006-000001")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        shop.transactions
            .find(&branch(MAIN), "XC-20261006-000002")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        shop.transactions
            .list(&branch(MAIN), Mode::Live)
            .await
            .unwrap(),
        vec![stored]
    );
    assert_eq!(
        shop.transactions
            .list(&branch(MAIN), Mode::Demo)
            .await
            .unwrap(),
        vec![]
    );

    // the receipt is a file of whoever saved, in that branch, named by its own hash
    let receipt = shop
        .files
        .open_owned(&receipt_key, OWNER)
        .await
        .unwrap()
        .expect("the receipt is a stored file");
    assert_eq!(receipt.mime, "text/html");
    assert_eq!(receipt.branch_id, MAIN);
    assert_eq!(receipt.created_at, millis(at(0)));
    let hash = sha256(&shop.pool, &receipt.bytes).await;
    assert_eq!(receipt_key, format!("receipt-{hash}"));
    assert_eq!(receipt.sha256, hash);
    let html = String::from_utf8(receipt.bytes).expect("a receipt is text");
    assert!(
        html.contains("XC-20261006-000001") && html.contains("Somchai J. Jaidee"),
        "{html}"
    );
    // staff of that branch can open it too
    assert!(
        shop.files
            .open_in_branch(&receipt_key, &branch(MAIN))
            .await
            .unwrap()
            .is_some()
    );
    shop.finish().await;
}

#[tokio::test]
async fn whoever_saves_is_recorded_and_must_own_the_files() {
    let shop = Shop::start().await;
    let (customer, owners_passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let staff = "employee-7";
    let staff_passport = shop.upload(MAIN, staff, "image/jpeg", JPEG).await;

    // a file somebody else uploaded is not this saver's attachment
    let got = shop
        .workflow
        .save(
            &branch(MAIN),
            form("staff-0001-aaaaaaaaaa", &customer, &owners_passport),
            staff,
            at(0),
        )
        .await;
    assert_eq!(refusal(got), ATTACHMENT_NOT_FOUND);

    let saved = shop
        .workflow
        .save(
            &branch(MAIN),
            form("staff-0001-aaaaaaaaaa", &customer, &staff_passport),
            staff,
            at(1),
        )
        .await
        .expect("the staff member's own file");
    let stored = shop
        .transactions
        .find(&branch(MAIN), &saved.document_ref)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.recorded_by, staff);
    let receipt_key = saved.receipt_url.strip_prefix("/files/").unwrap();
    assert!(
        shop.files
            .open_owned(receipt_key, staff)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        shop.files
            .open_owned(receipt_key, OWNER)
            .await
            .unwrap()
            .is_none()
    );

    // the owner sends the same submission with the same passport bytes of the
    // owner's own: it is the same exchange, and the first answer comes back
    let again = shop
        .workflow
        .save(
            &branch(MAIN),
            form("staff-0001-aaaaaaaaaa", &customer, &owners_passport),
            OWNER,
            at(2),
        )
        .await
        .expect("the same exchange sent again");
    assert_eq!(again, saved);
    assert_eq!(shop.count("transactions").await, 1);
    shop.finish().await;
}

// ------------------------------------------------------- the attached files

#[tokio::test]
async fn the_attached_files_are_judged_before_everything_but_the_branch_name() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    shop.seed_branch("XC-B01", "Second").await;
    let slip = shop.upload(MAIN, OWNER, "application/pdf", PDF).await;
    let elsewhere = shop.upload("XC-B01", OWNER, "image/jpeg", JPEG).await;
    let someone_elses = shop.upload(MAIN, "employee-7", "image/jpeg", JPEG).await;
    let gone = shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;
    std::fs::remove_file(shop.dir.join(&gone)).expect("remove an uploaded file");
    let never = "0f8fad5b-d9cb-469f-a165-70867728950e".to_owned();
    let before = shop.everything().await;

    let transfer = |passport: &str, slip: &str| {
        let mut cmd = form("files-0001-aaaaaaaaaa", &customer, passport);
        cmd.method = "Bank Transfer".to_owned();
        cmd.payment_ref = "TRX-1".to_owned();
        cmd.slip = Some(slip.to_owned());
        cmd
    };

    // a key that cannot be an upload: 36 characters of 0-9 a-f and the dash
    for bad in [
        String::new(),
        "x".to_owned(),
        passport.to_uppercase(),
        format!("{passport}0"),
        passport[..35].to_owned(),
        format!("{}g", &passport[..35]),
        format!(" {}", &passport[..35]),
        format!("../{}", &passport[..33]),
        format!("receipt-{}", "a".repeat(64)),
    ] {
        assert_eq!(
            refusal(
                shop.save(MAIN, form("files-0001-aaaaaaaaaa", &customer, &bad), at(0))
                    .await
            ),
            BAD_ATTACHMENT,
            "passport {bad:?}"
        );
        assert_eq!(
            refusal(shop.save(MAIN, transfer(&passport, &bad), at(0)).await),
            BAD_ATTACHMENT,
            "slip {bad:?}"
        );
    }
    // a well-formed key that is not this saver's upload in this branch
    for (what, key) in [
        ("never uploaded", &never),
        ("uploaded in another branch", &elsewhere),
        ("uploaded by someone else", &someone_elses),
    ] {
        assert_eq!(
            refusal(
                shop.save(MAIN, form("files-0001-aaaaaaaaaa", &customer, key), at(0))
                    .await
            ),
            ATTACHMENT_NOT_FOUND,
            "passport {what}"
        );
        assert_eq!(
            refusal(shop.save(MAIN, transfer(&passport, key), at(0)).await),
            ATTACHMENT_NOT_FOUND,
            "slip {what}"
        );
    }
    // recorded, but the bytes are not on the disk
    assert_eq!(
        refusal(
            shop.save(MAIN, form("files-0001-aaaaaaaaaa", &customer, &gone), at(0))
                .await
        ),
        ATTACHMENT_NOT_READY
    );
    assert_eq!(
        refusal(shop.save(MAIN, transfer(&passport, &gone), at(0)).await),
        ATTACHMENT_NOT_READY
    );

    // the passport is judged before the slip
    assert_eq!(
        refusal(shop.save(MAIN, transfer(&never, "x"), at(0)).await),
        ATTACHMENT_NOT_FOUND
    );
    assert_eq!(
        refusal(shop.save(MAIN, transfer("x", &never), at(0)).await),
        BAD_ATTACHMENT
    );
    assert_eq!(
        refusal(shop.save(MAIN, transfer(&gone, &never), at(0)).await),
        ATTACHMENT_NOT_READY
    );

    // a bad file is told before a bad form, a wrong customer, a wrong rate …
    let mut cmd = form("files-0001-aaaaaaaaaa", "CX-99999999", &never);
    cmd.customer.name = String::new();
    cmd.rate = "1".to_owned();
    assert_eq!(
        refusal(shop.save(MAIN, cmd, at(0)).await),
        ATTACHMENT_NOT_FOUND
    );
    // … and before the branch is looked up: a file is never "of" a branch that is not there
    assert_eq!(
        refusal(
            shop.save(
                "XC-B99",
                form("files-0001-aaaaaaaaaa", &customer, &passport),
                at(0)
            )
            .await
        ),
        ATTACHMENT_NOT_FOUND
    );
    // a form that names no file gets as far as the branch, then the form
    let mut no_files = form("files-0001-aaaaaaaaaa", &customer, &passport);
    no_files.passport = None;
    assert_eq!(
        refusal(shop.save("XC-B99", no_files.clone(), at(0)).await),
        NO_SUCH_BRANCH
    );
    assert_eq!(
        refusal(shop.save(MAIN, no_files, at(0)).await),
        FILE_MISSING
    );
    // only a branch name that cannot be one comes first
    for bad in ["", "XC_B", "สาขา", &"a".repeat(71)] {
        assert_eq!(
            refusal(
                shop.save(bad, form("files-0001-aaaaaaaaaa", &customer, &never), at(0))
                    .await
            ),
            BAD_BRANCH,
            "branch {bad:?}"
        );
    }

    assert_eq!(
        shop.everything().await,
        before,
        "a refused save changed something"
    );
    // and with good files it is saved
    let saved = shop
        .save(MAIN, transfer(&passport, &slip), at(0))
        .await
        .expect("good files");
    assert_eq!(saved.slip_url, format!("/files/{slip}"));
    shop.finish().await;
}

// ------------------------------------------------------- all of it or none

#[tokio::test]
async fn a_save_that_fails_part_way_leaves_nothing_behind() {
    // Whatever order the writes are made in, a failure of any one of them must
    // undo the others: the number, the stock, the exchange, the receipt's
    // record, the customer.
    for table in [
        "counters",
        "shift_balances",
        "files",
        "transactions",
        "transaction_files",
        "customers",
    ] {
        let shop = Shop::start().await;
        let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
        let mut cmd = form("broken-0001-aaaaaaaaa", &customer, &passport);
        cmd.customer.name = "A New Name".to_owned();
        cmd.customer.birth_date = "1990-01-31".to_owned();
        let before = shop.everything().await;

        shop.break_writes(table).await;
        let got = shop.save(MAIN, cmd.clone(), at(0)).await;
        assert!(
            matches!(&got, Err(e) if !matches!(e, pos_error::AppError::Domain(_))),
            "writes to {table} fail: expected an error that is not a refusal, got {got:?}"
        );
        shop.mend(table).await;
        assert_eq!(
            shop.everything().await,
            before,
            "writes to {table} failed, yet something was kept"
        );
        assert_eq!(
            shop.balances(MAIN).await.unwrap()["THB"],
            "100000.00",
            "{table}"
        );

        // nothing is left in the way: the same submission is saved, and is number 1
        let saved = shop
            .save(MAIN, cmd, at(1))
            .await
            .unwrap_or_else(|e| panic!("after mending {table}: {e:?}"));
        assert_eq!(saved.document_ref, "XC-20261006-000001", "{table}");
        assert_eq!(
            shop.balances(MAIN).await.unwrap()["THB"],
            "96790.00",
            "{table}"
        );
        shop.finish().await;
    }
}

#[tokio::test]
async fn a_refused_save_leaves_nothing_behind() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "3210"), ("USD", "10")]).await;
    let before = shop.everything().await;
    let mut cmd = form("refused-0001-aaaaaaaa", &customer, &passport);
    cmd.customer.name = "Never Written".to_owned();

    let mut too_much = cmd.clone();
    too_much.amount = "100.01".to_owned();
    assert_eq!(
        refusal(shop.save(MAIN, too_much, at(0)).await),
        NOT_ENOUGH_THB
    );
    let mut wrong_rate = cmd.clone();
    wrong_rate.rate = "32.2".to_owned();
    assert_eq!(
        refusal(shop.save(MAIN, wrong_rate, at(0)).await),
        RATE_MISMATCH
    );
    let mut sell = cmd.clone();
    sell.side = "SELL".to_owned();
    sell.rate = "33".to_owned();
    sell.amount = "10.01".to_owned();
    assert_eq!(refusal(shop.save(MAIN, sell, at(0)).await), NOT_ENOUGH_USD);
    let mut stranger = cmd.clone();
    stranger.customer_id = "CX-99999999".to_owned();
    assert_eq!(
        refusal(shop.save(MAIN, stranger, at(0)).await),
        NOT_THIS_PASSPORT
    );
    shop.close_shift(MAIN).await;
    let closed = shop.everything().await;
    assert_eq!(
        refusal(shop.save(MAIN, cmd.clone(), at(0)).await),
        NO_OPEN_SHIFT
    );
    // Re-pinned for TX-6 by the test-writer, decision A3 (design quote-transaction.md §4.6.1, #674;
    // lane controller's ruling, Fact 3 option (i)): the save is quoted first, so a wrong rate and a
    // currency the shop does not trade are told before the missing shift
    let mut wrong_rate = cmd.clone();
    wrong_rate.rate = "1".to_owned();
    assert_eq!(
        refusal(shop.save(MAIN, wrong_rate, at(0)).await),
        RATE_MISMATCH,
        "decision A3: the quote comes first"
    );
    let mut no_rate = cmd;
    no_rate.currency = "EUR".to_owned();
    assert_eq!(
        refusal(shop.save(MAIN, no_rate, at(0)).await),
        RATE_MISMATCH,
        "decision A3: the quote comes first"
    );
    assert_eq!(shop.everything().await, closed);
    assert_eq!(
        closed, before,
        "closing a shift does not touch what a save writes"
    );
    assert_eq!(shop.count("transactions").await, 0);
    shop.finish().await;
}

// ------------------------------------------------------------ at the same time

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn one_submission_sent_many_times_at_once_is_one_exchange() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let mut tasks = Vec::new();
    for i in 0..12 {
        let workflow = shop.workflow.clone();
        let cmd = form("at-once-0001-aaaaaaaa", &customer, &passport);
        tasks.push(tokio::spawn(async move {
            workflow.save(&branch(MAIN), cmd, OWNER, at(i)).await
        }));
    }
    let mut answers = Vec::new();
    for task in tasks {
        let got = tokio::time::timeout(MUST_FINISH, task)
            .await
            .expect("saves finish")
            .expect("no save panics");
        answers.push(got.expect("every one of them is answered as saved"));
    }
    assert!(answers.iter().all(|a| a == &answers[0]), "{answers:?}");
    assert_eq!(answers[0].document_ref, "XC-20261006-000001");
    assert_eq!(shop.count("transactions").await, 1);
    assert_eq!(
        shop.counters().await,
        BTreeMap::from([("document-LIVE-20261006".to_owned(), "1".to_owned())])
    );
    // the stock moved once
    let balances = shop.balances(MAIN).await.unwrap();
    assert_eq!(
        (balances["THB"].as_str(), balances["USD"].as_str()),
        ("96790.00", "1100.00")
    );
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn saves_at_once_never_sell_more_than_the_shift_holds() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "0"), ("USD", "50")]).await;
    let mut tasks = Vec::new();
    for i in 0..12 {
        let workflow = shop.workflow.clone();
        let mut cmd = form(&format!("stock-{i:04}-aaaaaaaaaa"), &customer, &passport);
        cmd.side = "SELL".to_owned();
        cmd.rate = "33".to_owned();
        cmd.amount = "10".to_owned();
        tasks.push(tokio::spawn(async move {
            workflow.save(&branch(MAIN), cmd, OWNER, at(i)).await
        }));
    }
    let mut refs = BTreeSet::new();
    let mut refused = 0;
    for task in tasks {
        match tokio::time::timeout(MUST_FINISH, task)
            .await
            .expect("saves finish")
            .expect("no save panics")
        {
            Ok(saved) => assert!(
                refs.insert(saved.document_ref.clone()),
                "{} twice",
                saved.document_ref
            ),
            Err(pos_error::AppError::Domain(sentence)) => {
                assert_eq!(sentence, NOT_ENOUGH_USD);
                refused += 1;
            }
            Err(other) => panic!("neither saved nor refused: {other:?}"),
        }
    }
    // fifty dollars are five sales of ten: no more, no fewer, numbered 1 to 5
    let want: BTreeSet<String> = (1..=5).map(|n| format!("XC-20261006-{n:06}")).collect();
    assert_eq!(refs, want);
    assert_eq!(refused, 7);
    let balances = shop.balances(MAIN).await.unwrap();
    assert_eq!(
        balances.get("USD"),
        None,
        "every dollar is sold: {balances:?}"
    );
    assert_eq!(balances["THB"], "1650.00");
    assert_eq!(shop.count("transactions").await, 5);
    shop.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn two_branches_saving_at_once_share_one_run_of_numbers() {
    // the document number is counted for the whole shop, but each branch has its
    // own lock: the counter itself has to keep two branches apart
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    shop.seed_branch("XC-B01", "Second").await;
    shop.set_rate("XC-B01", "USD", "32.1", "33").await;
    shop.open_shift("XC-B01", &[("THB", "100000"), ("USD", "1000")])
        .await;
    let customer_b = shop.reserve("XC-B01", person()).await;
    let passport_b = shop.upload("XC-B01", OWNER, "image/jpeg", JPEG).await;

    let mut tasks = Vec::new();
    for i in 0..16 {
        for (branch_id, customer, passport) in [
            (MAIN, &customer, &passport),
            ("XC-B01", &customer_b, &passport_b),
        ] {
            let workflow = shop.workflow.clone();
            let mut cmd = form(&format!("both-{i:04}-aaaaaaaaaaa"), customer, passport);
            cmd.amount = "1".to_owned();
            tasks.push(tokio::spawn(async move {
                workflow.save(&branch(branch_id), cmd, OWNER, at(i)).await
            }));
        }
    }
    let mut refs = BTreeSet::new();
    for task in tasks {
        let saved = tokio::time::timeout(MUST_FINISH, task)
            .await
            .expect("saves finish")
            .expect("no save panics")
            .expect("every save is saved");
        assert!(
            refs.insert(saved.document_ref.clone()),
            "{} was given twice",
            saved.document_ref
        );
    }
    let want: BTreeSet<String> = (1..=32).map(|n| format!("XC-20261006-{n:06}")).collect();
    assert_eq!(refs, want, "32 numbers, none twice, none skipped");
    assert_eq!(shop.refs(MAIN, Mode::Live).await.len(), 16);
    assert_eq!(shop.refs("XC-B01", Mode::Live).await.len(), 16);
    for branch_id in [MAIN, "XC-B01"] {
        let balances = shop.balances(branch_id).await.unwrap();
        assert_eq!(
            (balances["THB"].as_str(), balances["USD"].as_str()),
            ("99486.40", "1016.00"),
            "{branch_id}"
        );
    }
    shop.finish().await;
}
