//! Whether the board may show a published line (pricing-tiers.md §15.7, B5; V9:pricing `automaticBoardProjection`).
//! Fail closed for FOLLOW; `MANUAL` is never hidden; `AUTO` is never hidden by this rule. Pure.
//!
//! SKELETON (XC-V9-PRICING): `FollowVisibility::of` and `HiddenReason::as_str` are PV-i's. Types and signatures stay.

use uuid::Uuid;

use crate::domain::pricing::follow_plan::FollowResultStatus;
use crate::domain::pricing::tier_set::PricingSource;

/// Why a line is hidden (§15.7 `hiddenReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HiddenReason {
    Stale,
    Blocked,
    Mismatch,
}

impl HiddenReason {
    /// `"STALE"` · `"BLOCKED"` · `"MISMATCH"`.
    pub fn as_str(self) -> &'static str {
        todo!("PV-i: HiddenReason::as_str")
    }
}

/// What `FollowVisibility::of` reads for one published line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibilityInput {
    /// The line's `source_snapshot.pricing.source`, read through `TierSetSource::pricing_source`; `Auto` when absent.
    pub line_source: PricingSource,
    /// The line's `source_snapshot.pricing.tier_set_id`.
    pub line_tier_set_id: Option<Uuid>,
    /// That tier set's `base_tier_set_id` (a followed branch's row points at MAIN's set).
    pub line_base_tier_set_id: Option<Uuid>,
    /// The line's branch is MAIN.
    pub at_main: bool,
    /// `SourceFreshness::is_current` of the line's tier set's anchor source, now.
    pub source_current: bool,
    /// The branch's latest `FOLLOW` tier set for the currency.
    pub branch_latest_follow: Option<Uuid>,
    /// MAIN's latest `FOLLOW` tier set for the currency.
    pub main_latest_follow: Option<Uuid>,
    /// MAIN's latest `pricing_follow_results` row: its status and `tier_set_id`.
    pub main_result: Option<(FollowResultStatus, Option<Uuid>)>,
}

pub struct FollowVisibility;

impl FollowVisibility {
    /// `None` = visible. For a `Follow` line, the first that holds, in this order:
    /// - `Stale`: `!source_current`;
    /// - `Blocked`: MAIN's latest result is `Blocked`;
    /// - `Mismatch`: the line's tier set is not `branch_latest_follow`; or MAIN's latest result is not `Published` with
    ///   MAIN's set (the line's own set at MAIN, its `base` at a followed branch); or, at a followed branch, the base is
    ///   not `main_latest_follow`.
    ///
    /// `Manual` and `Auto` lines: always `None`.
    pub fn of(input: &VisibilityInput) -> Option<HiddenReason> {
        let _ = (input,);
        todo!("PV-i: FollowVisibility::of")
    }
}
