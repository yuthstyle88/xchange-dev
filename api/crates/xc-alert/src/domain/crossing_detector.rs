//! R5–R7: crossing, episode and cooldown. Pure: no database, no clock of its own.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::condition::AlertCondition;
use super::relation::Relation;
use super::source_policy::Observation;

/// What the detector needs to know of one alert, and what it changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertState {
    pub condition: AlertCondition,
    /// THB per `unit_size`, scale ≤ 8.
    pub target: Decimal,
    pub cooldown_seconds: i32,
    pub relation: Relation,
    /// The current or last episode (R6); 0 before the first crossing.
    pub episode_no: i32,
    pub last_triggered_at: Option<DateTime<Utc>>,
}

/// A crossing outside the cooldown: write one event (and its notification) for this episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fire {
    pub episode_no: i32,
    pub observed_rate: Decimal,
    /// The observation's time (the publication's `published_at`), which becomes `last_triggered_at`.
    pub trigger_at: DateTime<Utc>,
}

/// Turns one observation into the next state and, on a crossing outside the cooldown, a [`Fire`].
pub struct CrossingDetector;

impl CrossingDetector {
    /// `now` is the observation's time (the publication's `published_at`).
    pub fn observe(
        mut state: AlertState,
        observation: Observation,
        now: DateTime<Utc>,
    ) -> (AlertState, Option<Fire>) {
        let rate = match observation {
            Observation::Compare(r) => r,
            Observation::Skip => return (state, None),
        };
        let is_satisfied = state.condition.satisfied(rate, state.target);
        let new_relation = if is_satisfied {
            Relation::Satisfied
        } else {
            Relation::NotSatisfied
        };

        match state.relation {
            Relation::Unknown => {
                state.relation = new_relation;
                (state, None)
            }
            Relation::NotSatisfied => {
                if new_relation == Relation::Satisfied {
                    state.episode_no += 1;
                    state.relation = Relation::Satisfied;
                    let in_cooldown = match state.last_triggered_at {
                        Some(last) => {
                            now < last
                                + chrono::Duration::seconds(i64::from(state.cooldown_seconds))
                        }
                        None => false,
                    };
                    if in_cooldown {
                        (state, None)
                    } else {
                        state.last_triggered_at = Some(now);
                        let fire = Fire {
                            episode_no: state.episode_no,
                            observed_rate: rate,
                            trigger_at: now,
                        };
                        (state, Some(fire))
                    }
                } else {
                    (state, None)
                }
            }
            Relation::Satisfied => {
                if new_relation == Relation::NotSatisfied {
                    state.relation = Relation::NotSatisfied;
                }
                (state, None)
            }
        }
    }
}
