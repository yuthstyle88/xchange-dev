//! The ledger (LG-1 #923; design `docs/design/blueprint/ledger-inventory.md` §3.1, §8): cash accounts,
//! balanced postings and their entries. Nothing posts until LG-2/LG-4.

pub mod account;
pub mod posting;
pub mod repository;
