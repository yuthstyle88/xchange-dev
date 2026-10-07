//! The gap of one family of T11a part two: `saveTransaction`.
//!
//! `true` (ticket P2-S): the family's file
//! (`api/src/application/compat/actions/save.rs`) answers `saveTransaction`
//! on both routes, and the recorded steps of the name are held to what v2
//! answered. Set in P2-S's test commit, before the arm exists: until the arm
//! lands, the replays that record a save are red.

pub const WIRED: bool = true;
