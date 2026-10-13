//! `CaseActor`: who runs a customer-side command (design V3, V17). A branch reviewer may open, add and submit for a
//! customer who is present; neither reaches a reviewer state by these steps.

use super::customer_actor::CustomerActor;
use super::reviewer_actor::ReviewerActor;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaseActor {
    Customer(CustomerActor),
    Reviewer(ReviewerActor),
}
