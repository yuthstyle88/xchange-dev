//! RF-1: the one freshness rule (§4.10).

use chrono::{DateTime, NaiveDate, Utc};
use xc_kernel::money::Side;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Freshness {
    pub stale: bool,
    pub source_date: Option<NaiveDate>,
}

impl Freshness {
    /// One side of one line, from its `source_snapshot` and the side ("buy" / "sell").
    pub fn of_side(snapshot: &serde_json::Value, side: Side, now: DateTime<Utc>) -> Freshness {
        let _ = side; // today both sides share the line's snapshot keys
        let source_date = snapshot
            .get("source_date")
            .and_then(|v| v.as_str())
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());

        let fetched_at = snapshot
            .get("fetched_at")
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok());

        let max_age_hours = snapshot
            .get("max_reference_age_hours")
            .and_then(|v| v.as_i64());

        let stale = match (fetched_at, max_age_hours) {
            (Some(fat), Some(max_hours)) => {
                let age = now.signed_duration_since(fat.with_timezone(&Utc));
                age.num_seconds() > max_hours * 3600
            }
            _ => false,
        };

        Freshness { stale, source_date }
    }

    /// A whole publication: stale if any side is; the oldest source_date.
    pub fn of_publication(sides: impl IntoIterator<Item = Freshness>) -> Freshness {
        let mut stale = false;
        let mut oldest_date: Option<NaiveDate> = None;

        for f in sides {
            if f.stale {
                stale = true;
            }
            if let Some(d) = f.source_date {
                oldest_date = Some(match oldest_date {
                    Some(cur) => cur.min(d),
                    None => d,
                });
            }
        }

        Freshness {
            stale,
            source_date: oldest_date,
        }
    }
}
