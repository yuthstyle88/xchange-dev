//! The gap of one family of T11a part two: `documents`.
//!
//! `true` since ticket T11a-D: the family's file
//! (`api/src/application/compat/actions/documents.rs`) answers `documents` on
//! the staff route, so the recorded steps of that name are held to what v2
//! answered (`rpc_test.rs`: `rpc/engine_actions.json` #127, #128) and the name
//! is no longer part of the gap test. Its own tests are `rpc_documents_test.rs`.

pub const WIRED: bool = true;
