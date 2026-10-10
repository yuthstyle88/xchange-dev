//! QU-A2 (#556 design rows QU-3, QU-4, QU-5) — the desk's own rules, no database: `CallOrder`
//! (BOOKING first, then the lowest sequence), the counter's states, and `WaitEstimator` (the
//! sample, `percentile_cont`, and the estimate with its range). Written before the implementation
//! by the ticket author (claude/b060e9). Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! Design: `docs/design/blueprint/queue-tickets.md` Q6, Q7, Q9, §9.1 rows QU-3/QU-4/QU-5.
//! Owner Q-A2 (booking line first), Q-A4 (pause), Q-A5 (the sample), Q-A6 (the range).
//!
//! Signatures this test assumes (QU-A2 scaffolding, `[required]` once pushed):
//! - `CallOrder::{rank(ServiceType) -> u8, cmp(&QueueTicket, &QueueTicket) -> Ordering}`;
//! - `CounterStatus::{as_str, parse}`, `Counter::can_call() -> Result<(), QueueError>`;
//! - `WaitEstimator::{TODAY_MINIMUM, EARLIER_SAMPLE, sample, percentile_cont, estimate}`,
//!   `WaitEstimate { seconds, low_seconds, high_seconds }`.

use chrono::{NaiveDate, TimeZone, Utc};
use xc_kernel::ids::BranchId;
use xc_queue::domain::counter::counter::{Counter, CounterStatus};
use xc_queue::domain::status::wait_estimator::{WaitEstimate, WaitEstimator};
use xc_queue::domain::ticket::call_order::CallOrder;
use xc_queue::domain::ticket::queue_error::QueueError;
use xc_queue::domain::ticket::queue_ticket::{QueueTicket, TicketStatus};
use xc_queue::domain::ticket::service_type::ServiceType;

fn waiting(service_type: ServiceType, sequence: i32) -> QueueTicket {
    let letter = match service_type {
        ServiceType::WalkIn => 'W',
        ServiceType::Booking => 'B',
    };
    QueueTicket {
        id: format!("{letter}{sequence}"),
        organization_id: "00000000-0000-0000-0000-000000000001".to_owned(),
        branch_id: BranchId::new("MAIN"),
        service_date: NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
        service_type,
        sequence,
        display_number: format!("{letter}{sequence:03}"),
        status: TicketStatus::Waiting,
        customer_id: None,
        booking_id: (service_type == ServiceType::Booking).then(|| format!("BK-{sequence}")),
        kiosk_request_id: None,
        counter_teller_id: None,
        issued_at: Utc.with_ymd_and_hms(2026, 11, 2, 3, 0, 0).unwrap(),
        called_at: None,
        served_at: None,
        completed_at: None,
        ended_reason: None,
        idempotency_key: format!("k-{letter}{sequence}"),
    }
}

#[test]
fn the_booking_line_is_called_first_then_the_lowest_sequence() {
    assert!(
        CallOrder::rank(ServiceType::Booking) < CallOrder::rank(ServiceType::WalkIn),
        "BOOKING before WALK_IN (owner Q-A2)"
    );
    let mut tickets = [
        waiting(ServiceType::WalkIn, 10),
        waiting(ServiceType::Booking, 2),
        waiting(ServiceType::WalkIn, 1),
        waiting(ServiceType::WalkIn, 2),
        waiting(ServiceType::Booking, 1),
    ];
    tickets.sort_by(CallOrder::cmp);
    let order: Vec<&str> = tickets.iter().map(|t| t.display_number.as_str()).collect();
    assert_eq!(order, vec!["B001", "B002", "W001", "W002", "W010"]);
    assert_eq!(
        CallOrder::cmp(
            &waiting(ServiceType::Booking, 99),
            &waiting(ServiceType::WalkIn, 1)
        ),
        std::cmp::Ordering::Less,
        "a booking ticket with a high number still goes before walk-in 1"
    );
    assert_eq!(
        CallOrder::cmp(
            &waiting(ServiceType::WalkIn, 3),
            &waiting(ServiceType::WalkIn, 3)
        ),
        std::cmp::Ordering::Equal
    );
}

#[test]
fn counter_states_as_stored_and_a_paused_counter_does_not_call() {
    assert_eq!(CounterStatus::Active.as_str(), "ACTIVE");
    assert_eq!(CounterStatus::Paused.as_str(), "PAUSED");
    assert_eq!(CounterStatus::parse("ACTIVE"), Some(CounterStatus::Active));
    assert_eq!(CounterStatus::parse("PAUSED"), Some(CounterStatus::Paused));
    assert_eq!(CounterStatus::parse("active"), None);
    assert_eq!(CounterStatus::parse("CLOSED"), None);

    let counter = |status| Counter {
        organization_id: "00000000-0000-0000-0000-000000000001".to_owned(),
        branch_id: BranchId::new("MAIN"),
        teller_id: "00000000-0000-0000-0000-0000000000b1".to_owned(),
        status,
    };
    assert_eq!(counter(CounterStatus::Active).can_call(), Ok(()));
    assert_eq!(
        counter(CounterStatus::Paused).can_call(),
        Err(QueueError::CounterPaused),
        "a PAUSED counter cannot call (owner Q-A4)"
    );
}

#[test]
fn percentile_cont_interpolates_as_postgresql_does() {
    let s = [60, 120, 180, 240];
    assert_eq!(WaitEstimator::percentile_cont(&s, 0.25), Some(105.0));
    assert_eq!(WaitEstimator::percentile_cont(&s, 0.5), Some(150.0));
    assert_eq!(WaitEstimator::percentile_cont(&s, 0.75), Some(195.0));
    assert_eq!(
        WaitEstimator::percentile_cont(&[240, 60, 180, 120], 0.25),
        Some(105.0),
        "the order of the sample does not matter"
    );
    assert_eq!(
        WaitEstimator::percentile_cont(&[60, 120, 180], 0.5),
        Some(120.0)
    );
    assert_eq!(
        WaitEstimator::percentile_cont(&[60, 120, 180], 0.25),
        Some(90.0)
    );
    assert_eq!(WaitEstimator::percentile_cont(&[10, 20], 0.5), Some(15.0));
    assert_eq!(WaitEstimator::percentile_cont(&[100], 0.25), Some(100.0));
    assert_eq!(WaitEstimator::percentile_cont(&[100], 0.75), Some(100.0));
    assert_eq!(WaitEstimator::percentile_cont(&[], 0.5), None);
}

#[test]
fn the_sample_is_todays_once_it_has_three_else_the_earlier_one() {
    assert_eq!(WaitEstimator::TODAY_MINIMUM, 3);
    assert_eq!(WaitEstimator::EARLIER_SAMPLE, 30);
    let earlier = [40, 80, 120];
    assert_eq!(
        WaitEstimator::sample(&[10, 20], &earlier),
        &earlier[..],
        "2 today: the earlier sample (owner Q-A5)"
    );
    assert_eq!(
        WaitEstimator::sample(&[10, 20, 30], &earlier),
        &[10, 20, 30][..],
        "3 today: today's"
    );
    assert_eq!(WaitEstimator::sample(&[], &earlier), &earlier[..]);
    assert_eq!(WaitEstimator::sample(&[], &[]), &[] as &[i64]);
    assert_eq!(
        WaitEstimator::sample(&[10, 20], &[]),
        &[] as &[i64],
        "2 today and none before: no sample"
    );
}

#[test]
fn the_estimate_is_the_tickets_ahead_over_the_counters_times_each_percentile() {
    let four = [60, 120, 180, 240];
    let range = |seconds, low_seconds, high_seconds| {
        Some(WaitEstimate {
            seconds,
            low_seconds,
            high_seconds,
        })
    };
    assert_eq!(
        WaitEstimator::estimate(2, 1, &four, &[]),
        range(300, 210, 390),
        "design §9.1 QU-5: 2 ahead on 1 counter = 2×150, low 2×105, high 2×195 (owner Q-A6)"
    );
    assert_eq!(
        WaitEstimator::estimate(4, 2, &four, &[]),
        range(300, 210, 390),
        "4 ahead shared by 2 counters"
    );
    assert_eq!(
        WaitEstimator::estimate(1, 1, &[60, 120, 180], &four),
        range(120, 90, 150),
        "today's 3"
    );
    let thirty: Vec<i64> = (1..=30).map(|k| 40 * k).collect();
    assert_eq!(
        WaitEstimator::estimate(1, 1, &[10, 20], &thirty),
        range(620, 330, 910),
        "2 today: the earlier 30"
    );
    assert_eq!(
        WaitEstimator::estimate(0, 1, &four, &[]),
        range(0, 0, 0),
        "nobody ahead: no wait"
    );
}

#[test]
fn the_estimate_is_unknown_without_a_sample_or_an_active_counter() {
    assert_eq!(
        WaitEstimator::estimate(2, 0, &[60, 120, 180], &[]),
        None,
        "no active counter"
    );
    assert_eq!(
        WaitEstimator::estimate(2, 1, &[], &[]),
        None,
        "no completed ticket today or before"
    );
    assert_eq!(
        WaitEstimator::estimate(2, 1, &[10, 20], &[]),
        None,
        "2 today, none before: unknown"
    );
}

#[test]
fn low_is_never_above_the_estimate_and_the_estimate_never_above_high() {
    let samples: [&[i64]; 4] = [
        &[5, 500, 7, 9000],
        &[60],
        &[100, 100, 100],
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
    ];
    for sample in samples {
        for (ahead, counters) in [(1, 1), (3, 2), (7, 3), (12, 5)] {
            let e = WaitEstimator::estimate(ahead, counters, sample, &[])
                .expect("a sample and a counter");
            assert!(
                e.low_seconds <= e.seconds && e.seconds <= e.high_seconds,
                "{sample:?}, {ahead} ahead, {counters} counters: {e:?}"
            );
        }
    }
}
