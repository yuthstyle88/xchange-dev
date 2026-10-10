//! OB-1 up-front tests — `RetryPolicy` and `OutboxStatus` (design broker-seam.md §5.1, §5.2).
//! No database. Written before the implementation by the ticket author. Do not edit, delete
//! or weaken; add tests in a file of your own.

use std::time::Duration;

use xc_outbox::domain::event::OutboxStatus;
use xc_outbox::domain::retry_policy::RetryPolicy;

#[test]
fn the_default_is_5_seconds_doubling_to_10_minutes_and_7_days_to_expire() {
    assert_eq!(
        RetryPolicy::default(),
        RetryPolicy {
            first: Duration::from_secs(5),
            cap: Duration::from_secs(600),
            expire_after: Duration::from_secs(7 * 24 * 3600),
        }
    );
}

#[test]
fn backoff_doubles_from_first_and_stops_at_the_cap() {
    let p = RetryPolicy::default();
    let secs: Vec<u64> = (1..=9).map(|a| p.backoff(a).as_secs()).collect();
    assert_eq!(secs, vec![5, 10, 20, 40, 80, 160, 320, 600, 600]);
}

#[test]
fn backoff_never_overflows() {
    let p = RetryPolicy::default();
    for a in [64, 1_000, i32::MAX] {
        assert_eq!(p.backoff(a), Duration::from_secs(600), "attempts = {a}");
    }
}

#[test]
fn backoff_follows_a_custom_policy() {
    let p = RetryPolicy {
        first: Duration::from_secs(60),
        cap: Duration::from_secs(3600),
        expire_after: Duration::from_secs(86_400),
    };
    let secs: Vec<u64> = (1..=8).map(|a| p.backoff(a).as_secs()).collect();
    assert_eq!(secs, vec![60, 120, 240, 480, 960, 1920, 3600, 3600]);
}

#[test]
fn every_status_round_trips_through_its_stored_text() {
    let all = [
        (OutboxStatus::Pending, "PENDING"),
        (OutboxStatus::Delivered, "DELIVERED"),
        (OutboxStatus::Failed, "FAILED"),
        (OutboxStatus::Expired, "EXPIRED"),
    ];
    for (status, text) in all {
        assert_eq!(status.as_str(), text);
        assert_eq!(OutboxStatus::parse(text), Some(status));
    }
    assert_eq!(OutboxStatus::parse("pending"), None);
    assert_eq!(OutboxStatus::parse(""), None);
}
