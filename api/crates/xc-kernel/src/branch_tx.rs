//! `BranchTx` — per-branch transaction wrapper.
//!
//! Wraps `pos_infra::Transactor` to take `SELECT … FOR UPDATE` on that
//! branch's `branch_versions` row first, increments the version inside the
//! transaction, and commits. Implementation is a later ticket.
