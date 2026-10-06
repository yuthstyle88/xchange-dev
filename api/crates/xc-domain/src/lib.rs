//! XCHANGE business rules with no I/O. Design: `docs/design/xchange-v3.md` §3 and §5.
//!
//! Every module is declared here from the start so that tickets working on
//! different modules never touch this file.

pub mod custody;
pub mod kiosk;
pub mod money;
pub mod policy;
pub mod quote;
pub mod stock;
