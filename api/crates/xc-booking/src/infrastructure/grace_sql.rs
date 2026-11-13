//! The one SQL form of the no-show grace end (owner-set-values.md §3 row 19).
pub const GRACE_UNTIL_SQL: &str = "(s.start_at + (p.no_show_grace_minutes * interval '1 minute'))";
