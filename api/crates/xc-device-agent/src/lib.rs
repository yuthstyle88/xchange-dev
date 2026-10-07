//! The counter drawer agent of XCHANGE (`docs/design/xchange-v3.md` §5.4c): it pulls drawer
//! jobs of one branch from the cloud server and sends each kick to one printer.
//!
//! STUB written by the ticket author of row AG-5: the names and signatures below are the
//! ticket's [required] ones; every body answers a wrong value until the implementer writes it.
pub mod client;
pub mod clock;
pub mod config;
pub mod cups_transport;
pub mod journal;
pub mod runner;
pub mod tcp_transport;
pub mod transport;
