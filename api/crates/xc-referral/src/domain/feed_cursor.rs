//! `FeedCursor` (design §6.4): the keyset position `(updated_at, transaction_id)` as Broker's
//! `CpCursor` writes it, `<RFC 3339 time>_<hyphenated uuid>`.

use chrono::{DateTime, SecondsFormat, Timelike, Utc};
use uuid::Uuid;

/// A cursor that does not parse; the route answers `400`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedCursor {
    /// UTC, at microsecond precision (the precision of `timestamptz`).
    pub updated_at: DateTime<Utc>,
    pub id: Uuid,
}

impl FeedCursor {
    /// Split at the **first** `_`. The time is any RFC 3339 timestamp (0 to 9 fractional digits,
    /// `Z` or a numeric offset), converted to UTC and truncated to microseconds. The id is a
    /// hyphenated UUID, the nil UUID included. Anything else is `Err(CursorError)`.
    pub fn parse(text: &str) -> Result<Self, CursorError> {
        let (time_part, id_part) = text.split_once('_').ok_or(CursorError)?;
        if time_part.is_empty() || id_part.len() != 36 {
            return Err(CursorError);
        }
        let id_bytes = id_part.as_bytes();
        if id_bytes[8] != b'-'
            || id_bytes[13] != b'-'
            || id_bytes[18] != b'-'
            || id_bytes[23] != b'-'
        {
            return Err(CursorError);
        }
        let id = Uuid::parse_str(id_part).map_err(|_| CursorError)?;

        let dt = DateTime::parse_from_rfc3339(time_part).map_err(|_| CursorError)?;
        let utc = dt.with_timezone(&Utc);
        let nanos = (utc.nanosecond() / 1_000) * 1_000;
        let updated_at = utc.with_nanosecond(nanos).ok_or(CursorError)?;

        Ok(Self { updated_at, id })
    }

    /// `to_rfc3339_opts(SecondsFormat::Micros, true)` + `_` + the hyphenated id, exactly as
    /// Broker's `CpCursor::encode`.
    pub fn encode(&self) -> String {
        format!(
            "{}_{}",
            self.updated_at.to_rfc3339_opts(SecondsFormat::Micros, true),
            self.id
        )
    }
}
