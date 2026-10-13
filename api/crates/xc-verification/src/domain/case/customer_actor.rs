//! `CustomerActor` (design V3, §3): the signed-in customer acting on their own case (`Actor.customer_id`).

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomerActor {
    pub customer_id: String,
}
