use xc_queue::domain::status::wait_estimator::{WaitEstimate, WaitEstimator};

#[test]
fn row_1_percentile_cont_empty_and_single_element() {
    assert_eq!(WaitEstimator::percentile_cont(&[], 0.5), None);
    assert_eq!(WaitEstimator::percentile_cont(&[42], 0.25), Some(42.0));
}

#[test]
fn row_2_percentile_cont_sorts_unsorted_input() {
    assert_eq!(
        WaitEstimator::percentile_cont(&[240, 60, 180, 120], 0.5),
        Some(150.0)
    );
}

#[test]
fn row_3_percentile_cont_at_upper_bound_p_one() {
    assert_eq!(WaitEstimator::percentile_cont(&[10, 20], 1.0), Some(20.0));
}

#[test]
fn row_4_sample_fallback_and_today_minimum_boundary() {
    assert_eq!(WaitEstimator::sample(&[1, 2], &[5]), &[5]);
    assert_eq!(WaitEstimator::sample(&[1, 2], &[]), &[] as &[i64]);
    assert_eq!(WaitEstimator::sample(&[1, 2, 3], &[5]), &[1, 2, 3]);
}

#[test]
fn row_5_estimate_rejects_negative_active_counters() {
    assert_eq!(WaitEstimator::estimate(1, -1, &[100, 200, 300], &[]), None);
}

#[test]
fn row_6_estimate_uses_fractional_ahead_over_counters_ratio() {
    assert_eq!(
        WaitEstimator::estimate(3, 2, &[100, 200, 300, 400, 500], &[]),
        Some(WaitEstimate {
            seconds: 450,
            low_seconds: 300,
            high_seconds: 600,
        })
    );
}
