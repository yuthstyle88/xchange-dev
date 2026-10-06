//! The custody machine: who may hand cash to whom, in which phase, and what it records.
//!
//! Provided with ticket T9. These tests are the specification: make them pass,
//! do not edit, delete or weaken them.
//!
//! The first part reads as the rules. The second part holds the machine to every answer
//! the v2 engine gave (`fixtures/v2_custody_cases.json`, written by
//! `api/tools/v2-fixtures/gen_custody.mjs`).

mod custody_common;

use custody_common::*;
use xc_kernel::money::Side;
use xc_shift::domain::shift::value_objects::stock_delta::Direction;

use CustodyCommand as Cmd;
use CustodyError as E;

fn clock_in() -> Cmd {
    Cmd::Clock(ClockDirection::In)
}

fn clock_out() -> Cmd {
    Cmd::Clock(ClockDirection::Out)
}

fn begin() -> Cmd {
    Cmd::BeginTransaction {
        side: Side::Buy,
        currency: code("USD"),
        amount: money("100"),
    }
}

fn start(d: &Desk) -> Cmd {
    Cmd::StartService {
        equipment_ready: true,
        rates_verified: true,
        rate_updated: d.rates_updated.clone(),
    }
}

fn propose(p: Proposal) -> Cmd {
    Cmd::Propose(Box::new(p))
}

fn sign(id: &str) -> Cmd {
    Cmd::Sign { id: id.into() }
}

fn dispatch(id: &str) -> Cmd {
    Cmd::Dispatch { id: id.into() }
}

fn commit(id: &str) -> Cmd {
    Cmd::Commit { id: id.into() }
}

fn reject(id: &str) -> Cmd {
    Cmd::Reject {
        id: id.into(),
        reason: "count again".into(),
    }
}

fn reconcile(id: &str) -> Cmd {
    Cmd::Reconcile {
        id: id.into(),
        note: "matched the bank slip".into(),
    }
}

fn close(n: u32, drawer: &[(&str, &str)], safe: &[(&str, &str)]) -> Proposal {
    Proposal {
        counted: sheet(drawer),
        safe_count: sheet(safe),
        ..proposal(RequestKind::Close, n)
    }
}

fn kinds(events: &[CustodyEvent]) -> Vec<EventKind> {
    events.iter().map(|e| e.kind).collect()
}

fn refused(got: Result<Transition, E>) -> E {
    got.expect_err("must be refused")
}

// ── setting up ───────────────────────────────────────────────────────────────

#[test]
fn a_branch_starts_disabled_closed_and_empty() {
    let s = CustodyState::initial();
    assert!(!s.enabled);
    assert_eq!(s.phase, Phase::Closed);
    assert_eq!(
        (s.safe.is_none(), s.target.is_none(), s.entity.is_none()),
        (true, true, true)
    );
    assert_eq!(s.custodian, None);
    assert!(s.attendance.is_empty() && s.requests.is_empty() && s.holds.is_empty());
    assert!(s.drafts.is_empty() && s.exceptions.is_empty() && s.cutoff.is_none());
}

#[test]
fn the_clock_is_read_in_bangkok() {
    let t = |hhmm| CustodyTime::from_epoch_millis(bangkok(hhmm)).bangkok_minute_of_day();
    assert_eq!(CustodyTime::from_epoch_millis(NOON).epoch_millis(), NOON);
    assert_eq!(t("12:00"), 720);
    assert_eq!(t("00:00"), 0);
    assert_eq!(t("06:59"), 419); // the day before in UTC
    assert_eq!(t("23:59"), 1439);
    assert_eq!(
        CustodyTime::from_epoch_millis(0).bangkok_minute_of_day(),
        420
    );
    assert_eq!(
        CustodyTime::from_epoch_millis(-1).bangkok_minute_of_day(),
        419
    );
}

#[test]
fn only_the_owner_names_the_business_and_enables_and_in_that_order() {
    let mut d = Desk::new();
    let entity = || Cmd::SetEntity {
        company: " XCHANGE Co., Ltd. ".into(),
        site: "Main".into(),
        confirm_separated: true,
    };
    assert_eq!(refused(d.act("owner", Cmd::Enable)), E::EntityMissing);
    assert_eq!(refused(d.act("F1", entity())), E::RoleNotAllowed);
    assert_eq!(refused(d.act("F1", Cmd::Enable)), E::RoleNotAllowed);
    let t = d.ok("owner", entity());
    assert_eq!(kinds(&t.events), [EventKind::SetEntity]);
    let e = d.state.entity.clone().unwrap();
    assert_eq!(
        (e.company.as_str(), e.site.as_str(), e.confirmed_by.as_str()),
        ("XCHANGE Co., Ltd.", "Main", "owner-1")
    );
    assert!(!d.state.enabled);
    let t = d.ok("owner", Cmd::Enable);
    assert_eq!(kinds(&t.events), [EventKind::Enable]);
    assert!(d.state.enabled);
    assert_eq!(refused(d.act("owner", entity())), E::EntityLocked);
}

#[test]
fn the_business_must_be_confirmed_in_full() {
    let mut d = Desk::new();
    let with = |company: &str, site: &str, confirm| Cmd::SetEntity {
        company: company.into(),
        site: site.into(),
        confirm_separated: confirm,
    };
    for cmd in [
        with("  ", "Main", true),
        with("Co", "", true),
        with("Co", "Main", false),
        with(&"c".repeat(151), "Main", true),
        with("Co", &"s".repeat(101), true),
    ] {
        assert_eq!(refused(d.act("owner", cmd)), E::EntityIncomplete);
    }
    d.ok("owner", with(&"c".repeat(150), &"s".repeat(100), true));
}

#[test]
fn custody_is_not_set_up_or_enabled_over_an_open_shift() {
    let mut d = Desk::new();
    d.shift = Some(OpenShift {
        id: "S0".into(),
        expected: bal(&[]),
    });
    let entity = Cmd::SetEntity {
        company: "Co".into(),
        site: "Main".into(),
        confirm_separated: true,
    };
    assert_eq!(refused(d.act("owner", entity.clone())), E::EntityLocked);
    assert_eq!(refused(d.act("owner", Cmd::Enable)), E::ShiftStillOpen);
    d.shift = None;
    d.ok("owner", entity);
    d.ok("owner", Cmd::Enable);
}

#[test]
fn until_the_owner_enables_custody_every_other_command_is_refused() {
    let mut d = Desk::new();
    for cmd in [
        clock_in(),
        begin(),
        Cmd::Cutoff,
        sign("x"),
        propose(proposal(RequestKind::Open, 1)),
    ] {
        assert_eq!(refused(d.act("F1", cmd)), E::NotEnabled);
    }
}

// ── attendance ───────────────────────────────────────────────────────────────

#[test]
fn clocking_in_twice_is_one_attendance_and_one_event() {
    let mut d = Desk::enabled();
    let before = d.state.attendance.len();
    let t = d.ok("F1", clock_in());
    assert!(t.events.is_empty());
    assert_eq!(d.state.attendance.len(), before);
    let mine: Vec<_> = d
        .state
        .attendance
        .iter()
        .filter(|a| a.employee_id == "F1")
        .collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(
        (mine[0].role, mine[0].clock_out),
        (CustodyRole::Front, None)
    );
}

#[test]
fn a_new_attendance_takes_the_id_it_was_given() {
    let mut d = Desk::enabled();
    d.ok("F1", clock_out());
    let id = d.facts().new_id;
    let t = d.ok("F1", clock_in());
    assert_eq!(kinds(&t.events), [EventKind::ClockIn]);
    let a = d.state.attendance.last().unwrap();
    assert_eq!((a.id.as_str(), a.employee_id.as_str()), (id.as_str(), "F1"));
    assert_eq!(a.clock_in, CustodyTime::from_epoch_millis(d.now));
}

#[test]
fn the_owner_has_no_attendance_and_nobody_clocks_out_who_is_not_in() {
    let mut d = Desk::enabled();
    assert_eq!(refused(d.act("owner", clock_in())), E::EmployeeRequired);
    assert_eq!(refused(d.act("F3", clock_out())), E::NoClockInToClose);
    let t = d.ok("F2", clock_out());
    assert_eq!(kinds(&t.events), [EventKind::ClockOut]);
    assert_eq!(refused(d.act("F2", clock_out())), E::NoClockInToClose);
}

#[test]
fn whoever_is_not_clocked_in_can_do_nothing_else() {
    let mut d = Desk::open();
    assert_eq!(refused(d.act("F3", begin())), E::NotClockedIn);
    assert_eq!(refused(d.act("owner", Cmd::Cutoff)), E::NotClockedIn);
    d.ok("BE2", clock_out());
    assert_eq!(refused(d.act("BE2", Cmd::Cutoff)), E::NotClockedIn);
}

#[test]
fn the_custodian_and_the_parties_of_a_pending_request_stay_clocked_in() {
    let (mut d, _) = Desk::transfer(RequestKind::ExternalIn);
    assert_eq!(refused(d.act("F1", clock_out())), E::DutiesOutstanding); // custodian and sender
    assert_eq!(refused(d.act("FIN1", clock_out())), E::DutiesOutstanding); // receiver
    d.ok("FIN2", clock_out());
}

// ── opening ──────────────────────────────────────────────────────────────────

#[test]
fn the_safe_gets_its_opening_count_from_finance_monitor_and_backend() {
    let mut d = Desk::enabled();
    let count = Proposal {
        counted: sheet(SAFE),
        ..proposal(RequestKind::SafeOpen, 1)
    };
    assert_eq!(
        refused(d.act("F1", propose(count.clone()))),
        E::RoleNotAllowed
    );
    let new_id = d.facts().new_id;
    let t = d.ok("FIN1", propose(count));
    let id = t.id.clone().unwrap();
    assert_eq!(id, new_id);
    assert_eq!(
        t.events,
        [CustodyEvent {
            kind: EventKind::Propose,
            subject: Some(id.clone())
        }]
    );
    let r = &d.state.requests[0];
    assert_eq!(
        (r.kind, r.status, r.sender.as_str()),
        (RequestKind::SafeOpen, RequestStatus::Pending, "FIN1")
    );
    assert_eq!(r.signatures.len(), 1); // the sender signs by proposing
    assert_eq!(
        (r.signatures[0].employee_id.as_str(), r.signatures[0].role),
        ("FIN1", CustodyRole::Finance)
    );

    assert_eq!(refused(d.act("BE1", commit(&id))), E::AwaitingMonitor);
    assert_eq!(refused(d.act("MON1", commit(&id))), E::RoleNotAllowed);
    let t = d.ok("MON1", sign(&id));
    assert_eq!(
        t.events,
        [CustodyEvent {
            kind: EventKind::Sign,
            subject: Some(id.clone())
        }]
    );
    assert_eq!(d.state.safe, None);
    let t = d.commit("BE1", &id);
    assert_eq!(
        t.events,
        [CustodyEvent {
            kind: EventKind::Commit,
            subject: Some(id.clone())
        }]
    );
    assert!(t.effects.is_empty());
    assert_eq!(d.state.safe, Some(bal(SAFE)));
    let r = &d.state.requests[0];
    assert_eq!(
        (r.status, r.reconciliation),
        (RequestStatus::Completed, Some(Reconciliation::Pending))
    );
    assert_eq!(r.signatures.last().unwrap().role, CustodyRole::Backend); // the commit is Backend's signature
    assert!(d.state.exceptions.is_empty());

    let again = Proposal {
        counted: sheet(SAFE),
        ..proposal(RequestKind::SafeOpen, 9)
    };
    assert_eq!(refused(d.act("FIN1", propose(again))), E::SafeAlreadyOpened);
}

#[test]
fn opening_needs_a_live_rate_a_target_and_no_shift() {
    let mut d = Desk::enabled();
    let open = Proposal {
        counted: sheet(DRAWER),
        ..proposal(RequestKind::Open, 2)
    };
    assert_eq!(
        refused(d.act("F1", propose(open.clone()))),
        E::TargetMissing
    );
    d.has_live_rate = false;
    assert_eq!(refused(d.act("F1", propose(open.clone()))), E::NoLiveRate);
    d.has_live_rate = true;
    let t = d.ok(
        "FIN1",
        Cmd::SetTarget {
            amounts: bal(DRAWER),
        },
    );
    assert_eq!(kinds(&t.events), [EventKind::CashTarget]);
    assert_eq!(
        refused(d.act(
            "F1",
            Cmd::SetTarget {
                amounts: bal(DRAWER)
            }
        )),
        E::RoleNotAllowed
    );
    d.shift = Some(OpenShift {
        id: "S0".into(),
        expected: bal(&[]),
    });
    assert_eq!(
        refused(d.act("F1", propose(open.clone()))),
        E::ShiftAlreadyOpen
    );
    d.shift = None;
    assert_eq!(
        refused(d.act("FIN1", propose(open.clone()))),
        E::RoleNotAllowed
    );
    d.ok("F1", propose(open));
    assert_eq!(d.state.phase, Phase::Closed); // nothing moves until Backend commits
}

#[test]
fn committing_the_opening_count_opens_the_shift_and_makes_the_sender_custodian() {
    let mut d = Desk::with_safe();
    let id = d.propose(
        "F2",
        Proposal {
            counted: sheet(DRAWER),
            ..proposal(RequestKind::Open, 2)
        },
    );
    // No signature but Backend's own is asked for an opening.
    let t = d.commit("BE1", &id);
    assert_eq!(
        t.effects,
        [ShiftEffect::OpenShift {
            request_id: Some(request_id(2)),
            opening: bal(DRAWER),
        }]
    );
    assert_eq!(d.state.phase, Phase::Ready);
    assert_eq!(d.state.custodian.as_deref(), Some("F2"));
    assert!(d.state.exceptions.is_empty()); // the count met the target
    // A second commit is a repeat: accepted, nothing happens.
    let t = d.commit("BE1", &id);
    assert!(t.events.is_empty() && t.effects.is_empty());
    assert_eq!(d.state.requests[1].signatures.len(), 2);
}

#[test]
fn a_count_that_misses_the_target_needs_a_note_and_becomes_an_exception() {
    let mut d = Desk::with_safe();
    let short = Proposal {
        counted: sheet(&[("THB", "50000"), ("USD", "999.99")]),
        ..proposal(RequestKind::Open, 2)
    };
    assert_eq!(
        refused(d.act("F1", propose(short.clone()))),
        E::VarianceNeedsNote
    );
    let blank = Proposal {
        note: "   ".into(),
        ..short.clone()
    };
    assert_eq!(refused(d.act("F1", propose(blank))), E::VarianceNeedsNote);
    let id = d.propose(
        "F1",
        Proposal {
            note: "  one cent short  ".into(),
            ..short
        },
    );
    let r = d.state.requests.last().unwrap();
    assert_eq!(r.note, "one cent short");
    let diff = r.target_difference.clone().unwrap().non_zero();
    assert_eq!(diff.len(), 1);
    assert_eq!(
        (diff[0].0, diff[0].1.to_string()),
        (code("USD"), "-0.01".to_string())
    );
    let new_id = d.facts().new_id;
    d.commit("BE1", &id);
    let x = &d.state.exceptions[0];
    assert_eq!(
        (x.id.as_str(), x.request_id.as_str(), x.status),
        (new_id.as_str(), id.as_str(), ExceptionStatus::Open)
    );
    assert_eq!(x.note, "one cent short");
    assert!(x.cash_variance.is_none() && x.safe_variance.is_none());
}

#[test]
fn the_rows_of_a_count_must_be_sound_and_add_up() {
    let mut d = Desk::with_safe();
    let with = |rows: Vec<DenominationRow>| {
        propose(Proposal {
            counted: CountSheet {
                totals: bal(DRAWER),
                denominations: rows,
            },
            ..proposal(RequestKind::Open, 2)
        })
    };
    assert_eq!(refused(d.act("F1", with(vec![]))), E::DenominationsRequired);
    assert_eq!(
        refused(d.act("F1", with(vec![row("THB", "1", 1); 201]))),
        E::DenominationsRequired
    );
    for bad in [
        row("CHF", "100", 1), // only THB USD EUR GBP JPY CNY AUD are counted by denomination
        row("THB", "0", 1),   // a note of nothing
        row("THB", "100000.01", 1), // no such note
        row("THB", "1", 1_000_001), // too many of them
    ] {
        assert_eq!(
            refused(d.act("F1", with(vec![bad]))),
            E::DenominationInvalid
        );
    }
    // Rows are sound but the baht rows are missing: the first currency out is named, in
    // the order THB USD EUR GBP JPY CNY AUD.
    let usd_only = with(vec![row("USD", "100", 10)]);
    assert_eq!(
        refused(d.act("F1", usd_only)),
        E::DenominationMismatch(code("THB"))
    );
    let thb_only = with(vec![row("THB", "1000", 50)]);
    assert_eq!(
        refused(d.act("F1", thb_only)),
        E::DenominationMismatch(code("USD"))
    );
    // A row of zero pieces is allowed; 200 rows are allowed.
    let mut rows = vec![
        row("THB", "1000", 50),
        row("USD", "50", 20),
        row("JPY", "1000", 0),
    ];
    rows.extend(vec![row("EUR", "5", 0); 197]);
    d.ok("F1", with(rows));
}

#[test]
fn a_request_needs_a_well_formed_id_one_to_five_evidence_files_and_a_short_note() {
    let mut d = Desk::with_safe();
    let open = Proposal {
        counted: sheet(DRAWER),
        ..proposal(RequestKind::Open, 2)
    };
    let with = |f: &dyn Fn(&mut Proposal)| {
        let mut p = open.clone();
        f(&mut p);
        propose(p)
    };
    assert_eq!(
        refused(d.act("F1", with(&|p| p.request_id = "abc".into()))),
        E::RequestIdInvalid
    );
    assert_eq!(
        refused(d.act("F1", with(&|p| p.request_id = request_id(2).to_uppercase()))),
        E::RequestIdInvalid
    );
    assert_eq!(
        refused(d.act("F1", with(&|p| p.evidence_keys.clear()))),
        E::EvidenceRequired
    );
    assert_eq!(
        refused(d.act(
            "F1",
            with(&|p| p.evidence_keys = (1..=6).map(evidence).collect())
        )),
        E::EvidenceRequired
    );
    assert_eq!(
        refused(d.act("F1", with(&|p| p.evidence_keys = vec!["slip.jpg".into()]))),
        E::EvidenceRequired
    );
    assert_eq!(
        refused(d.act("F1", with(&|p| p.note = "x".repeat(501)))),
        E::NoteTooLong
    );
    d.ok(
        "F1",
        with(&|p| p.evidence_keys = (1..=5).map(evidence).collect()),
    );
}

#[test]
fn the_same_request_sent_again_returns_its_id_and_a_changed_one_is_refused() {
    let mut d = Desk::with_safe();
    let open = Proposal {
        counted: sheet(DRAWER),
        ..proposal(RequestKind::Open, 2)
    };
    let id = d.propose("F1", open.clone());
    let t = d.ok("F1", propose(open.clone()));
    assert_eq!(t.id.as_deref(), Some(id.as_str()));
    assert!(t.events.is_empty());
    assert_eq!(d.state.requests.len(), 2);
    let changed = Proposal {
        payload_hash: "another".into(),
        ..open
    };
    assert_eq!(refused(d.act("F1", propose(changed))), E::RequestChanged);
}

#[test]
fn one_request_is_pending_at_a_time() {
    let mut d = Desk::with_safe();
    let open = |n| {
        propose(Proposal {
            counted: sheet(DRAWER),
            ..proposal(RequestKind::Open, n)
        })
    };
    d.ok("F1", open(2));
    assert_eq!(refused(d.act("F2", open(3))), E::RequestPending);
    let id = d.request();
    d.ok("MON1", reject(&id));
    d.ok("F2", open(3));
}

// ── serving ──────────────────────────────────────────────────────────────────

#[test]
fn backend_opens_the_service_between_eleven_and_ten_to_eleven_at_night() {
    let mut d = Desk::ready();
    let go = start(&d);
    assert_eq!(refused(d.act("F1", go.clone())), E::RoleNotAllowed);
    assert_eq!(
        refused(d.act_at("BE1", go.clone(), bangkok("10:59"))),
        E::ServiceNotReady
    );
    assert_eq!(
        refused(d.act_at("BE1", go.clone(), bangkok("22:50"))),
        E::ServiceNotReady
    );
    let current = d.rates_updated.clone();
    for (ready, verified, updated) in [
        (false, true, current.as_str()),
        (true, false, current.as_str()),
        (true, true, "2026-10-05T03:00:00.000Z"),
    ] {
        let cmd = Cmd::StartService {
            equipment_ready: ready,
            rates_verified: verified,
            rate_updated: updated.into(),
        };
        assert_eq!(refused(d.act("BE1", cmd)), E::ServiceChecksIncomplete);
    }
    let t = d.act_at("BE1", go.clone(), bangkok("22:49")).unwrap();
    assert_eq!(kinds(&t.events), [EventKind::ServiceOpen]);
    assert_eq!(d.state.phase, Phase::Open);
    assert_eq!(
        d.state.service_opened_at,
        Some(CustodyTime::from_epoch_millis(bangkok("22:49")))
    );
    assert_eq!(refused(d.act("BE1", go)), E::ServiceNotReady); // only from Ready
}

#[test]
fn only_the_custodian_starts_a_transaction_and_only_while_the_service_is_open() {
    let mut d = Desk::ready();
    assert_eq!(refused(d.act("F1", begin())), E::NotCustodianInOpenService); // prepared, not open
    let go = start(&d);
    d.ok("BE1", go);
    assert_eq!(refused(d.act("F2", begin())), E::NotCustodianInOpenService);
    assert_eq!(refused(d.act("MON1", begin())), E::RoleNotAllowed);
    assert_eq!(
        refused(d.act_at("F1", begin(), bangkok("10:59"))),
        E::OutsideServiceHours
    );
    assert_eq!(
        refused(d.act_at("F1", begin(), bangkok("22:50"))),
        E::OutsideServiceHours
    );
    let thb = Cmd::BeginTransaction {
        side: Side::Buy,
        currency: code("THB"),
        amount: money("100"),
    };
    assert_eq!(refused(d.act("F1", thb)), E::DraftInvalid);
    let nothing = Cmd::BeginTransaction {
        side: Side::Sell,
        currency: code("USD"),
        amount: money("0"),
    };
    assert_eq!(refused(d.act("F1", nothing)), E::AmountInvalid);

    let new_id = d.facts().new_id;
    let t = d.act_at("F1", begin(), bangkok("22:49")).unwrap();
    assert_eq!(t.id.as_deref(), Some(new_id.as_str()));
    assert!(t.events.is_empty()); // a draft is not an event
    let draft = d.state.drafts.last().unwrap();
    assert_eq!(
        (
            draft.id.as_str(),
            draft.shift_id.as_str(),
            draft.employee_id.as_str()
        ),
        (new_id.as_str(), "S1", "F1")
    );
    assert_eq!(
        (draft.side, draft.currency, draft.amount, draft.status),
        (Side::Buy, code("USD"), money("100"), DraftStatus::Open)
    );
}

#[test]
fn a_transaction_is_not_started_without_a_shift() {
    // v2 crashed here (it read the id of a shift that was not there); v3 refuses.
    let mut d = Desk::open();
    d.shift = None;
    assert_eq!(refused(d.act("F1", begin())), E::NoOpenShift);
}

#[test]
fn backend_cuts_off_and_may_cut_off_again_but_not_while_counting() {
    let mut d = Desk::open();
    assert_eq!(refused(d.act("F1", Cmd::Cutoff)), E::RoleNotAllowed);
    d.pending_transactions = vec!["XC-7".into()];
    let t = d.ok("BE1", Cmd::Cutoff);
    assert_eq!(kinds(&t.events), [EventKind::CutOff]);
    assert_eq!(d.state.phase, Phase::CutOff);
    let c = d.state.cutoff.clone().unwrap();
    assert_eq!(
        (c.by.as_str(), c.shift_id.as_str(), c.pending.as_slice()),
        ("BE1", "S1", &["XC-7".to_string()][..])
    );
    assert_eq!(refused(d.act("F1", begin())), E::NotCustodianInOpenService);
    d.pending_transactions.clear();
    d.ok("BE2", Cmd::Cutoff);
    assert_eq!(d.state.cutoff.as_ref().unwrap().by, "BE2");
    assert!(d.state.cutoff.as_ref().unwrap().pending.is_empty());
    d.propose("F1", close(4, DRAWER, SAFE));
    assert_eq!(refused(d.act("BE1", Cmd::Cutoff)), E::CountingUnderWay);
    d.shift = None;
    assert_eq!(refused(d.act("BE1", Cmd::Cutoff)), E::NoOpenShift);
}

// ── closing and handing over ─────────────────────────────────────────────────

#[test]
fn the_drawer_is_counted_only_after_the_cut_off_by_its_custodian() {
    let mut d = Desk::open();
    assert_eq!(
        refused(d.act("F1", propose(close(4, DRAWER, SAFE)))),
        E::CutoffRequired
    );
    d.ok("BE1", Cmd::Cutoff);
    assert_eq!(
        refused(d.act("F2", propose(close(4, DRAWER, SAFE)))),
        E::NotDrawerCustodian
    );
    d.pending_transactions = vec!["XC-7".into()];
    assert_eq!(
        refused(d.act("F1", propose(close(4, DRAWER, SAFE)))),
        E::TransactionsPending
    );
    d.pending_transactions.clear();
    assert_eq!(d.state.phase, Phase::CutOff); // a refused proposal leaves the phase alone
    let id = d.propose("F1", close(4, DRAWER, SAFE));
    assert_eq!(d.state.phase, Phase::Counting);
    let r = d.state.requests.last().unwrap();
    assert_eq!(r.id, id);
    assert_eq!(r.shift_id.as_deref(), Some("S1"));
    assert_eq!(r.expected, Some(bal(DRAWER)));
    assert_eq!(r.safe_expected, Some(bal(SAFE)));
    assert!(r.variance.as_ref().unwrap().non_zero().is_empty());
    assert!(r.safe_variance.as_ref().unwrap().non_zero().is_empty());
    assert_eq!(r.cutoff, d.state.cutoff);
}

#[test]
fn closing_needs_manager_and_monitor_then_backend_closes_the_shift() {
    let mut d = Desk::cut_off();
    let id = d.propose("F1", close(4, DRAWER, SAFE));
    assert_eq!(refused(d.act("BE1", commit(&id))), E::AwaitingManager);
    d.sign("MGR1", &id);
    assert_eq!(refused(d.act("BE1", commit(&id))), E::AwaitingMonitor);
    d.sign("MON1", &id);
    assert_eq!(refused(d.act("F1", commit(&id))), E::RoleNotAllowed);
    let t = d.commit("BE1", &id);
    assert_eq!(
        t.effects,
        [ShiftEffect::CloseShift {
            request_id: request_id(4),
            shift_id: "S1".into(),
            expected: bal(DRAWER),
            counted: bal(DRAWER),
            note: String::new(),
        }]
    );
    assert_eq!(
        (d.state.phase, d.state.custodian.clone()),
        (Phase::Closed, None)
    );
    assert_eq!(d.state.safe, Some(bal(SAFE)));
    assert!(d.state.exceptions.is_empty());
    d.ok("F1", clock_out()); // no longer holds anything
}

#[test]
fn a_shortage_at_close_is_recorded_not_adjusted() {
    let mut d = Desk::cut_off();
    let drawer = [("THB", "49990"), ("USD", "1000")];
    let safe = [("THB", "200000"), ("USD", "5000"), ("EUR", "301")];
    assert_eq!(
        refused(d.act("F1", propose(close(4, &drawer, &safe)))),
        E::VarianceNeedsNote
    );
    let id = d.propose(
        "F1",
        Proposal {
            note: "ten baht short".into(),
            ..close(4, &drawer, &safe)
        },
    );
    d.sign("MGR1", &id);
    d.sign("MON1", &id);
    d.commit("BE1", &id);
    assert_eq!(d.state.safe, Some(bal(&safe))); // the safe is what was counted
    let x = &d.state.exceptions[0];
    let cash = x.cash_variance.clone().unwrap().non_zero();
    let safe_diff = x.safe_variance.clone().unwrap().non_zero();
    assert_eq!(
        (cash[0].0, cash[0].1.to_string()),
        (code("THB"), "-10.00".to_string())
    );
    assert_eq!(
        (safe_diff[0].0, safe_diff[0].1.to_string()),
        (code("EUR"), "1.00".to_string())
    );
    assert!(x.target_difference.is_none());

    // Finance reconciles: the exception is reviewed. Not by a party to the request.
    assert_eq!(refused(d.act("MON1", reconcile(&id))), E::RoleNotAllowed);
    let blank = Cmd::Reconcile {
        id: id.clone(),
        note: " ".into(),
    };
    assert_eq!(refused(d.act("FIN1", blank)), E::ReconcileNoteRequired);
    let t = d.ok("FIN1", reconcile(&id));
    assert_eq!(
        t.events,
        [CustodyEvent {
            kind: EventKind::Reconcile,
            subject: Some(id.clone())
        }]
    );
    let x = &d.state.exceptions[0];
    assert_eq!(
        (x.status, x.reviewed_by.as_deref()),
        (ExceptionStatus::Reviewed, Some("FIN1"))
    );
    let r = d.state.requests.last().unwrap();
    assert_eq!(r.reconciliation, Some(Reconciliation::Checked));
    assert_eq!(r.reconciled.as_ref().unwrap().text, "matched the bank slip");
}

#[test]
fn a_handover_needs_the_receiver_too_and_reopens_for_them() {
    let mut d = Desk::cut_off();
    let hand = |receiver: Option<&str>| {
        propose(Proposal {
            receiver: receiver.map(str::to_string),
            counted: sheet(DRAWER),
            safe_count: sheet(SAFE),
            ..proposal(RequestKind::Handover, 4)
        })
    };
    assert_eq!(refused(d.act("F1", hand(None))), E::HandoverReceiverInvalid);
    assert_eq!(
        refused(d.act("F1", hand(Some("F1")))),
        E::HandoverReceiverInvalid
    );
    assert_eq!(d.state.phase, Phase::CutOff);
    let id = d.ok("F1", hand(Some("F2"))).id.unwrap();
    d.sign("MGR1", &id);
    d.sign("MON1", &id);
    assert_eq!(refused(d.act("BE1", commit(&id))), E::AwaitingReceiver);
    d.sign("F2", &id);
    let t = d.commit("BE1", &id);
    assert_eq!(t.effects.len(), 2);
    assert!(matches!(&t.effects[0], ShiftEffect::CloseShift { shift_id, .. } if shift_id == "S1"));
    assert_eq!(
        t.effects[1],
        ShiftEffect::OpenShift {
            request_id: None, // a fresh one: the request's own id closed the old shift
            opening: bal(DRAWER),
        }
    );
    assert_eq!(
        (d.state.phase, d.state.custodian.as_deref()),
        (Phase::Open, Some("F2"))
    );
    d.ok("F2", begin());
    assert_eq!(d.state.drafts.last().unwrap().shift_id, "S2");
}

#[test]
fn a_count_is_void_when_the_drawer_or_the_shift_changed_before_the_commit() {
    let mut d = Desk::cut_off();
    let id = d.propose("F1", close(4, DRAWER, SAFE));
    d.sign("MGR1", &id);
    d.sign("MON1", &id);
    let counted_against = d.shift.clone();
    d.shift.as_mut().unwrap().expected = bal(&[("THB", "50000"), ("USD", "1100")]);
    assert_eq!(refused(d.act("BE1", commit(&id))), E::BalancesChanged);
    d.shift = Some(OpenShift {
        id: "S9".into(),
        expected: bal(DRAWER),
    });
    assert_eq!(refused(d.act("BE1", commit(&id))), E::ShiftChanged);
    d.shift = None;
    assert_eq!(refused(d.act("BE1", commit(&id))), E::ShiftChanged);
    d.shift = counted_against;
    d.commit("BE1", &id);
}

#[test]
fn sending_a_count_back_returns_to_the_cut_off() {
    let mut d = Desk::cut_off();
    let id = d.propose("F1", close(4, DRAWER, SAFE));
    assert_eq!(refused(d.act("F1", reject(&id))), E::RoleNotAllowed);
    let blank = Cmd::Reject {
        id: id.clone(),
        reason: "  ".into(),
    };
    assert_eq!(refused(d.act("MON1", blank)), E::RejectReasonRequired);
    let t = d.ok("MON1", reject(&id));
    assert_eq!(
        t.events,
        [CustodyEvent {
            kind: EventKind::Reject,
            subject: Some(id.clone())
        }]
    );
    assert_eq!(d.state.phase, Phase::CutOff);
    let r = d.state.requests.last().unwrap();
    assert_eq!(r.status, RequestStatus::Rejected);
    let why = r.rejected.as_ref().unwrap();
    assert_eq!(
        (why.by.as_str(), why.text.as_str()),
        ("MON1", "count again")
    );
    for cmd in [reject(&id), sign(&id), commit(&id)] {
        let who = if matches!(cmd, Cmd::Sign { .. }) {
            "MGR1"
        } else {
            "BE1"
        };
        assert_eq!(refused(d.act(who, cmd)), E::RequestFinished);
    }
    assert_eq!(refused(d.act("FIN1", reconcile(&id))), E::NotCompleted);
    d.propose("F1", close(5, DRAWER, SAFE)); // count again
}

// ── signatures ───────────────────────────────────────────────────────────────

#[test]
fn who_may_sign_what() {
    let mut d = Desk::cut_off();
    let id = d.propose(
        "F1",
        Proposal {
            receiver: Some("F2".into()),
            counted: sheet(DRAWER),
            safe_count: sheet(SAFE),
            ..proposal(RequestKind::Handover, 4)
        },
    );
    assert_eq!(refused(d.act("FIN1", sign(&id))), E::NotAFinanceRequest); // Finance signs transfers only
    assert_eq!(refused(d.act("BE1", sign(&id))), E::BackendCommitsInstead);
    assert_eq!(refused(d.act("RT1", sign(&id))), E::SignNotAllowed);
    assert_eq!(refused(d.act("F3", sign(&id))), E::NotClockedIn);
    assert_eq!(
        refused(d.act("MON1", sign("no-such-request"))),
        E::RequestNotFound
    );
    // The sender signed by proposing: signing again is a repeat, not a refusal.
    let t = d.ok("F1", sign(&id));
    assert!(t.events.is_empty());
    d.sign("F2", &id);
    d.sign("MON1", &id);
    d.sign("MGR1", &id);
    let t = d.ok("MGR1", sign(&id));
    assert!(t.events.is_empty());
    let who: Vec<_> = d
        .state
        .requests
        .last()
        .unwrap()
        .signatures
        .iter()
        .map(|s| (s.employee_id.as_str(), s.role))
        .collect();
    assert_eq!(
        who,
        [
            ("F1", CustodyRole::Front),
            ("F2", CustodyRole::Front),
            ("MON1", CustodyRole::Monitor),
            ("MGR1", CustodyRole::Manager)
        ]
    );
}

#[test]
fn a_front_signs_only_what_they_receive() {
    let mut d = Desk::cut_off();
    let id = d.propose("F1", close(4, DRAWER, SAFE));
    assert_eq!(refused(d.act("F2", sign(&id))), E::NotTheReceiver);
}

// ── transfers ────────────────────────────────────────────────────────────────

#[test]
fn a_transfer_names_one_of_seven_currencies_an_amount_and_another_person() {
    let mut d = Desk::open();
    let base = Proposal {
        receiver: Some("FIN1".into()),
        currency: Some(code("USD")),
        amount: Some(money("200")),
        ..proposal(RequestKind::ExternalIn, 3)
    };
    let with = |f: &dyn Fn(&mut Proposal)| {
        let mut p = base.clone();
        f(&mut p);
        propose(p)
    };
    assert_eq!(
        refused(d.act("F1", with(&|p| p.amount = None))),
        E::AmountMissing
    );
    assert_eq!(
        refused(d.act("F1", with(&|p| p.amount = Some(money("0"))))),
        E::AmountInvalid
    );
    assert_eq!(
        refused(d.act("F1", with(&|p| p.currency = None))),
        E::TransferPartiesInvalid
    );
    assert_eq!(
        refused(d.act("F1", with(&|p| p.currency = Some(code("CHF"))))),
        E::TransferPartiesInvalid
    );
    assert_eq!(
        refused(d.act("F1", with(&|p| p.receiver = None))),
        E::TransferPartiesInvalid
    );
    assert_eq!(
        refused(d.act("F1", with(&|p| p.receiver = Some("F1".into())))),
        E::TransferPartiesInvalid
    );
    assert_eq!(refused(d.act("F2", with(&|_| ()))), E::NotDrawerCustodian);
    d.shift = None;
    assert_eq!(refused(d.act("F1", with(&|_| ()))), E::NoOpenShift);
}

#[test]
fn money_for_the_safe_leaves_the_drawer_at_dispatch_and_reaches_the_safe_at_commit() {
    let (mut d, id) = Desk::transfer(RequestKind::SafeIn);
    assert_eq!(refused(d.act("FIN1", sign(&id))), E::AwaitingDispatch); // the receiver confirms a receipt
    assert_eq!(refused(d.act("BE1", commit(&id))), E::AwaitingFinance);
    assert_eq!(refused(d.act("F1", dispatch(&id))), E::RoleNotAllowed);
    let t = d.ok("BE1", dispatch(&id));
    assert_eq!(
        t.events,
        [CustodyEvent {
            kind: EventKind::Dispatch,
            subject: Some(id.clone())
        }]
    );
    assert_eq!(
        t.effects,
        [ShiftEffect::MoveStock {
            request_id: Some(request_id(3)),
            shift_id: "S1".into(),
            currency: code("USD"),
            direction: Direction::Out,
            amount: money("200"),
            reason: format!("IN_TRANSIT · SAFE_IN · {id}"),
        }]
    );
    let r = d.state.requests.last().unwrap();
    assert_eq!(
        (r.stage, r.dispatched_by.as_deref()),
        (Some(TransferStage::InTransit), Some("BE1"))
    );
    assert_eq!(d.state.safe, Some(bal(SAFE))); // not there yet
    let t = d.ok("BE2", dispatch(&id)); // a repeat
    assert!(t.events.is_empty() && t.effects.is_empty());
    assert_eq!(refused(d.act("MON1", reject(&id))), E::MoneyInTransit);
    assert_eq!(refused(d.act("F1", reject(&id))), E::MoneyInTransit); // said before the role is looked at

    d.sign("FIN1", &id); // receiver and Finance in one
    let t = d.commit("BE1", &id);
    assert!(t.effects.is_empty());
    assert_eq!(
        d.state.safe,
        Some(bal(&[("THB", "200000"), ("USD", "5200"), ("EUR", "300")]))
    );
    let r = d.state.requests.last().unwrap();
    assert_eq!(
        (r.status, r.stage),
        (RequestStatus::Completed, Some(TransferStage::Received))
    );
    assert_eq!(refused(d.act("BE1", dispatch(&id))), E::NotDispatchable);
    assert_eq!(refused(d.act("FIN1", reconcile(&id))), E::ReconcilerIsParty);
    d.ok("FIN2", reconcile(&id));
}

#[test]
fn money_from_the_safe_leaves_it_at_dispatch_and_reaches_the_drawer_at_commit() {
    let mut d = Desk::open();
    let id = d.propose(
        "F1",
        Proposal {
            receiver: Some("MGR1".into()),
            currency: Some(code("EUR")),
            amount: Some(money("300")),
            note: "float".into(),
            ..proposal(RequestKind::SafeOut, 3)
        },
    );
    let t = d.ok("BE1", dispatch(&id));
    assert!(t.effects.is_empty());
    assert_eq!(
        d.state.safe,
        Some(bal(&[("THB", "200000"), ("USD", "5000")]))
    ); // all 300 EUR left
    d.sign("MGR1", &id);
    assert_eq!(refused(d.act("BE1", commit(&id))), E::AwaitingFinance); // the receiver is not Finance
    d.sign("FIN2", &id);
    let t = d.commit("BE1", &id);
    assert_eq!(
        t.effects,
        [ShiftEffect::MoveStock {
            request_id: None,
            shift_id: "S1".into(),
            currency: code("EUR"),
            direction: Direction::In,
            amount: money("300"),
            reason: "SAFE_OUT · float".into(),
        }]
    );
}

#[test]
fn the_safe_cannot_give_more_than_it_holds() {
    let mut d = Desk::open();
    let id = d.propose(
        "F1",
        Proposal {
            receiver: Some("FIN1".into()),
            currency: Some(code("EUR")),
            amount: Some(money("300.01")),
            ..proposal(RequestKind::SafeOut, 3)
        },
    );
    assert_eq!(refused(d.act("BE1", dispatch(&id))), E::SafeInsufficient);
    assert_eq!(d.state.requests.last().unwrap().stage, None);
}

#[test]
fn money_to_and_from_outside_moves_the_drawer_only() {
    for (kind, at_dispatch, at_commit) in [
        (RequestKind::ExternalOut, 1, 0),
        (RequestKind::ExternalIn, 0, 1),
    ] {
        let (mut d, id) = Desk::transfer(kind);
        let t = d.ok("BE1", dispatch(&id));
        assert_eq!(t.effects.len(), at_dispatch, "{kind:?}");
        d.sign("FIN1", &id);
        let t = d.commit("BE1", &id);
        assert_eq!(t.effects.len(), at_commit, "{kind:?}");
        if let Some(ShiftEffect::MoveStock {
            reason, request_id, ..
        }) = t.effects.first()
        {
            assert_eq!(reason, &format!("EXTERNAL_IN · {id}")); // no note: the request id stands in
            assert_eq!(request_id, &None);
        }
        assert_eq!(d.state.safe, Some(bal(SAFE)), "{kind:?}");
    }
}

#[test]
fn a_transfer_is_dispatched_in_the_shift_it_was_proposed_in() {
    let (mut d, id) = Desk::transfer(RequestKind::ExternalOut);
    d.shift.as_mut().unwrap().id = "S9".into();
    assert_eq!(refused(d.act("BE1", dispatch(&id))), E::ShiftChanged);
    d.shift = None;
    assert_eq!(refused(d.act("BE1", dispatch(&id))), E::ShiftChanged);
}

#[test]
fn backend_does_not_commit_a_request_they_are_a_party_to() {
    let mut d = Desk::open();
    let id = d.propose(
        "F1",
        Proposal {
            receiver: Some("BE1".into()),
            currency: Some(code("USD")),
            amount: Some(money("1")),
            ..proposal(RequestKind::ExternalOut, 3)
        },
    );
    assert_eq!(refused(d.act("BE1", commit(&id))), E::BackendIsParty);
    assert_eq!(refused(d.act("BE2", commit(&id))), E::AwaitingFinance);
}

#[test]
fn only_counts_and_transfers_that_exist_can_be_acted_on() {
    let mut d = Desk::open();
    for cmd in [
        sign("nope"),
        dispatch("nope"),
        commit("nope"),
        reject("nope"),
        reconcile("nope"),
    ] {
        // Said to anyone clocked in, before the role is looked at.
        assert_eq!(refused(d.act("RT1", cmd)), E::RequestNotFound);
    }
    let count = d.state.requests[0].id.clone(); // the safe's opening count, completed
    assert_eq!(refused(d.act("BE1", dispatch(&count))), E::NotDispatchable);
}

// ── guards no sequence of commands reaches, kept because v2 has them ─────────

#[test]
fn guards_behind_other_guards_still_hold_on_a_state_loaded_from_storage() {
    // A transfer proposed while the phase says Counting but nothing is pending.
    let mut d = Desk::open();
    d.state.phase = Phase::Counting;
    let transfer = Proposal {
        receiver: Some("FIN1".into()),
        currency: Some(code("USD")),
        amount: Some(money("1")),
        ..proposal(RequestKind::ExternalIn, 3)
    };
    assert_eq!(
        refused(d.act("F1", propose(transfer))),
        E::ClosingCountUnderWay
    );

    // A transfer fully signed but never dispatched.
    let (mut d, id) = Desk::transfer(RequestKind::ExternalIn);
    let at = CustodyTime::from_epoch_millis(NOON);
    d.state
        .requests
        .last_mut()
        .unwrap()
        .signatures
        .push(Signature {
            employee_id: "FIN1".into(),
            role: CustodyRole::Finance,
            at,
        });
    assert_eq!(refused(d.act("BE1", commit(&id))), E::DispatchRequired);

    // A second opening count of the safe, pending while the safe already has one.
    let mut d = Desk::enabled();
    let id = d.propose(
        "FIN1",
        Proposal {
            counted: sheet(SAFE),
            ..proposal(RequestKind::SafeOpen, 1)
        },
    );
    d.sign("MON1", &id);
    d.state.safe = Some(bal(SAFE));
    assert_eq!(refused(d.act("BE1", commit(&id))), E::SafeAlreadyOpened);
}

// ── holding a customer ───────────────────────────────────────────────────────

#[test]
fn front_or_monitor_holds_a_customer_and_monitor_releases() {
    let mut d = Desk::open();
    let hold = |reason: &str| Cmd::Hold {
        customer_id: "CX-00000001".into(),
        reason: reason.into(),
    };
    assert_eq!(refused(d.act("FIN1", hold("x"))), E::RoleNotAllowed);
    assert_eq!(refused(d.act("F1", hold("  "))), E::HoldInvalid);
    d.customer_known = false;
    assert_eq!(refused(d.act("F1", hold("x"))), E::HoldInvalid);
    d.customer_known = true;
    let new_id = d.facts().new_id;
    let t = d.ok("F1", hold("passport photo unclear"));
    assert_eq!(
        t.events,
        [CustodyEvent {
            kind: EventKind::HoldCustomer,
            subject: Some("CX-00000001".into())
        }]
    );
    let h = d.state.holds[0].clone();
    assert_eq!(
        (h.id.as_str(), h.by.as_str(), h.status),
        (new_id.as_str(), "F1", HoldStatus::Open)
    );

    let release = |id: &str, note: &str| Cmd::Release {
        hold_id: id.into(),
        note: note.into(),
    };
    assert_eq!(
        refused(d.act("F1", release(&h.id, "ok"))),
        E::RoleNotAllowed
    );
    assert_eq!(
        refused(d.act("MON1", release(&h.id, " "))),
        E::ReleaseInvalid
    );
    assert_eq!(
        refused(d.act("MON1", release("nope", "ok"))),
        E::ReleaseInvalid
    );
    let t = d.ok("MON1", release(&h.id, "checked the original"));
    assert_eq!(
        t.events,
        [CustodyEvent {
            kind: EventKind::ReleaseCustomer,
            subject: Some(h.id.clone())
        }]
    );
    let h = &d.state.holds[0];
    assert_eq!(
        (h.status, h.reviewed_by.as_deref(), h.note.as_deref()),
        (
            HoldStatus::Released,
            Some("MON1"),
            Some("checked the original")
        )
    );
    assert_eq!(
        refused(d.act("MON1", release(&h.id.clone(), "again"))),
        E::ReleaseInvalid
    );
}

// ── the gate on a LIVE save (v2 server.mjs:6868-6877) ────────────────────────
// Not in the fixture: v2 reads the wall clock inside this check.

fn save(d: &Desk, draft: Option<&str>) -> SaveAttempt {
    let _ = d;
    SaveAttempt {
        customer_id: "CX-00000001".into(),
        draft_id: draft.map(str::to_string),
        side: Side::Buy,
        currency: code("USD"),
        amount: money("100"),
        prior: PriorSave::None,
    }
}

fn gate(d: &Desk, who: &str, s: &SaveAttempt, hhmm: &str) -> Result<(), E> {
    CustodyMachine::check_save(
        &d.state,
        s,
        &actor(who),
        &d.facts(),
        CustodyTime::from_epoch_millis(bangkok(hhmm)),
    )
}

#[test]
fn while_custody_is_off_a_save_is_not_gated() {
    let d = Desk::new();
    assert_eq!(gate(&d, "RT1", &save(&d, None), "03:00"), Ok(()));
}

#[test]
fn only_the_clocked_in_custodian_saves_while_the_service_is_open() {
    let mut d = Desk::open();
    let s = save(&d, None);
    assert_eq!(gate(&d, "F1", &s, "11:00"), Ok(()));
    assert_eq!(gate(&d, "F1", &s, "22:49"), Ok(()));
    assert_eq!(gate(&d, "F2", &s, "12:00"), Err(E::SaveNotCustodian));
    assert_eq!(gate(&d, "MGR1", &s, "12:00"), Err(E::SaveNotCustodian));
    assert_eq!(gate(&d, "owner", &s, "12:00"), Err(E::SaveNotCustodian));
    assert_eq!(gate(&d, "F1", &s, "10:59"), Err(E::SaveClosed));
    assert_eq!(gate(&d, "F1", &s, "22:50"), Err(E::SaveClosed)); // new work stops at 22:50 …
    assert_eq!(gate(&d, "F2", &s, "22:50"), Err(E::SaveClosed)); // the hours are said before the person
    d.state
        .attendance
        .iter_mut()
        .find(|a| a.employee_id == "F1")
        .unwrap()
        .clock_out = Some(CustodyTime::from_epoch_millis(NOON));
    assert_eq!(gate(&d, "F1", &s, "12:00"), Err(E::SaveNotCustodian));
}

#[test]
fn a_started_transaction_may_be_finished_after_the_cut_off_until_eleven() {
    let mut d = Desk::open();
    let draft = d.ok("F1", begin()).id.unwrap();
    let with = save(&d, Some(&draft));
    assert_eq!(gate(&d, "F1", &with, "22:50"), Ok(())); // … but a started one may finish
    assert_eq!(gate(&d, "F1", &with, "22:59"), Ok(()));
    assert_eq!(gate(&d, "F1", &with, "23:00"), Err(E::SaveClosed));
    d.ok("BE1", Cmd::Cutoff);
    assert_eq!(gate(&d, "F1", &with, "12:00"), Ok(()));
    assert_eq!(gate(&d, "F1", &save(&d, None), "12:00"), Err(E::SaveClosed));
    // The draft must be this employee's, this shift's, and for exactly this exchange.
    for other in [
        SaveAttempt {
            amount: money("100.01"),
            ..with.clone()
        },
        SaveAttempt {
            side: Side::Sell,
            ..with.clone()
        },
        SaveAttempt {
            currency: code("EUR"),
            ..with.clone()
        },
        SaveAttempt {
            draft_id: Some("nope".into()),
            ..with.clone()
        },
    ] {
        assert_eq!(gate(&d, "F1", &other, "12:00"), Err(E::SaveClosed));
    }
    let mut next_shift = Desk::open();
    next_shift.state = d.state.clone();
    next_shift.shift.as_mut().unwrap().id = "S2".into();
    assert_eq!(gate(&next_shift, "F1", &with, "12:00"), Err(E::SaveClosed));
    // While the count is on, and once closed, nothing is saved even with a draft.
    d.propose("F1", close(4, DRAWER, SAFE));
    assert_eq!(gate(&d, "F1", &with, "12:00"), Err(E::SaveClosed));

    // A used draft does not open the gate again.
    let mut d = Desk::open();
    let draft = d.ok("F1", begin()).id.unwrap();
    d.state = CustodyMachine::complete_draft(&d.state, &draft, "XC-20261006-000001");
    let done = d.state.drafts.last().unwrap();
    assert_eq!(
        (done.status, done.document_ref.as_deref()),
        (DraftStatus::Completed, Some("XC-20261006-000001"))
    );
    assert_eq!(
        gate(&d, "F1", &save(&d, Some(&draft)), "22:55"),
        Err(E::SaveClosed)
    );
    let same = CustodyMachine::complete_draft(&d.state, "nope", "XC-1");
    assert_eq!(same, d.state);
}

#[test]
fn a_customer_on_hold_is_not_served_until_released() {
    let mut d = Desk::open();
    d.ok(
        "MON1",
        Cmd::Hold {
            customer_id: "CX-00000001".into(),
            reason: "check".into(),
        },
    );
    let s = save(&d, None);
    assert_eq!(gate(&d, "F1", &s, "12:00"), Err(E::CustomerOnHold));
    assert_eq!(gate(&d, "F2", &s, "03:00"), Err(E::CustomerOnHold)); // said first
    let other = SaveAttempt {
        customer_id: "CX-00000002".into(),
        ..s.clone()
    };
    assert_eq!(gate(&d, "F1", &other, "12:00"), Ok(()));
    let hold = d.state.holds[0].id.clone();
    d.ok(
        "MON1",
        Cmd::Release {
            hold_id: hold,
            note: "fine".into(),
        },
    );
    assert_eq!(gate(&d, "F1", &s, "12:00"), Ok(()));
}

#[test]
fn a_retry_of_a_save_is_judged_by_what_is_already_stored() {
    let mut d = Desk::open();
    d.ok(
        "MON1",
        Cmd::Hold {
            customer_id: "CX-00000001".into(),
            reason: "check".into(),
        },
    );
    let stored = |prior| SaveAttempt {
        prior,
        ..save(&d, None)
    };
    // Already saved by this Front: let through whatever the hour, the phase or a hold —
    // the save itself will answer with the stored result.
    assert_eq!(
        gate(&d, "F1", &stored(PriorSave::SavedByActor), "03:00"),
        Ok(())
    );
    assert_eq!(
        gate(&d, "F2", &stored(PriorSave::SavedByActor), "03:00"),
        Ok(())
    ); // not asked who holds the drawer
    assert_eq!(
        gate(&d, "F1", &stored(PriorSave::SavedByOther), "12:00"),
        Err(E::SaveNotOwnRetry)
    );
    assert_eq!(
        gate(&d, "MGR1", &stored(PriorSave::SavedByActor), "12:00"),
        Err(E::SaveNotOwnRetry)
    );
    // Stored but not finished: the hours and the phase are not asked again, the rest is.
    assert_eq!(
        gate(&d, "F1", &stored(PriorSave::Unfinished), "03:00"),
        Err(E::CustomerOnHold)
    );
    let hold = d.state.holds[0].id.clone();
    d.ok(
        "MON1",
        Cmd::Release {
            hold_id: hold,
            note: "fine".into(),
        },
    );
    let stored = |prior| SaveAttempt {
        prior,
        ..save(&d, None)
    };
    assert_eq!(
        gate(&d, "F1", &stored(PriorSave::Unfinished), "03:00"),
        Ok(())
    );
    assert_eq!(
        gate(&d, "F2", &stored(PriorSave::Unfinished), "03:00"),
        Err(E::SaveNotCustodian)
    );
}

// ── every answer v2 gave ─────────────────────────────────────────────────────

/// Replays accepted and refused steps in order; a refused step leaves the state as it was.
fn replay(label: &str, steps: &[Step]) -> CustodyState {
    let mut state = CustodyState::initial();
    for (n, step) in steps.iter().enumerate() {
        let f = facts(&step.facts, &step.facts.new_id);
        let label = format!("{label}, step {n}");
        if let Some(next) = check(
            &label,
            &state,
            &step.actor,
            &step.command,
            &f,
            step.now,
            &step.answer,
        ) {
            state = next;
        }
    }
    state
}

#[test]
fn every_story_v2_played_plays_the_same() {
    let f = fixture();
    let (mut accepted, mut refused) = (0, 0);
    for story in &f.stories {
        replay(&story.name, &story.steps);
        refused += story
            .steps
            .iter()
            .filter(|s| s.answer.error.is_some())
            .count();
        accepted += story
            .steps
            .iter()
            .filter(|s| s.answer.error.is_none())
            .count();
    }
    assert_eq!((f.stories.len(), accepted, refused), (12, 275, 142));
}

#[test]
fn every_command_by_every_role_from_every_state_gets_v2s_answer() {
    let f = fixture();
    let (mut accepted, mut refused) = (0, 0);
    for m in &f.matrix {
        let state = replay(&m.state, &m.steps);
        assert_eq!(state.phase.as_str(), m.phase, "{}", m.state);
        for p in &m.probes {
            let facts = facts(&m.facts, &p.new_id);
            let got = check(
                &m.state,
                &state,
                &p.actor,
                &m.commands[p.command],
                &facts,
                m.now,
                &p.answer,
            );
            if got.is_some() {
                accepted += 1
            } else {
                refused += 1
            }
        }
    }
    assert_eq!((f.matrix.len(), accepted, refused), (15, 379, 3071));
}

#[test]
fn the_fixture_holds_the_cases_it_was_generated_with() {
    // A fixture regenerated with fewer cases must not pass unnoticed.
    let f = fixture();
    let c = &f.counts;
    assert_eq!(
        (c.matrix_states, c.probes, c.probes_accepted),
        (15, 3450, 379)
    );
    assert_eq!(
        (c.stories, c.story_steps, c.story_steps_refused),
        (12, 417, 142)
    );
    assert_eq!((c.distinct_errors, c.v2_differs_on_purpose), (62, 1));
    assert_eq!(
        f.matrix.iter().map(|m| m.probes.len()).sum::<usize>(),
        c.probes
    );
    assert_eq!(
        f.stories.iter().map(|s| s.steps.len()).sum::<usize>(),
        c.story_steps
    );
    // 23 commands × 10 actors from each state.
    assert!(
        f.matrix
            .iter()
            .all(|m| m.commands.len() == 23 && m.probes.len() == 230)
    );
    let phases: std::collections::BTreeSet<&str> =
        f.matrix.iter().map(|m| m.phase.as_str()).collect();
    assert_eq!(
        phases.into_iter().collect::<Vec<_>>(),
        ["CLOSED", "COUNTING", "CUT_OFF", "OPEN", "READY"]
    );
    // The one place v3 answers differently on purpose: v2 crashed.
    let crashed: Vec<&Step> = f
        .stories
        .iter()
        .flat_map(|s| &s.steps)
        .filter(|s| s.answer.v2_differs_on_purpose)
        .collect();
    assert_eq!(crashed.len(), 1);
    assert_eq!(crashed[0].answer.error.as_deref(), Some("ยังไม่มีกะเปิด"));
}
