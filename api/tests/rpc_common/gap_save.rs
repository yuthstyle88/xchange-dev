//! The gap of one family of T11a part two: `saveTransaction`.
//!
//! `false`: the family's file (`api/src/application/compat/actions/save.rs`)
//! answers no name yet, and the routes answer its names as names nobody knows
//! — which `rpc_test.rs` holds them to. The ticket that wires the family
//! (P2-S) sets this to `true` in the same change; from then on the recorded
//! steps of these names are held to what v2 answered.

pub const WIRED: bool = false;
