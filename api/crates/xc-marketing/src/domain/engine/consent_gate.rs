//! `ConsentGate` source trait (design §6, §7, D-13).

pub trait ConsentGate: Send + Sync {
    fn marketing_allowed(&self, customer: &str) -> bool;
}
